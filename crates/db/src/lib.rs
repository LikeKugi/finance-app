use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use finance_core::{
    canonical_amount, rebase_exchange_rate, validate_currency_code, validate_exchange_rate,
    AnalysisTransaction, Bank, Category, Currency, CurrencyDefinition, FinancialTransaction,
    InterestDestination, LinkedSavingsAccount, PaymentCardType, PaymentDocument,
    PaymentDocumentDetails, PaymentSystem, SavingsAccount, SavingsDetails, SavingsTerm,
    TransactionAmount, TransactionKind,
};
use rusqlite::backup::Backup;
use rusqlite::{
    params, types::Type, Connection, OpenFlags, OptionalExtension, TransactionBehavior,
};
use rust_decimal::Decimal;
use std::collections::HashMap;

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../migrations/V1__initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../migrations/V2__transactions_and_directories.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../migrations/V3__savings.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("../migrations/V4__payment_documents.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("../migrations/V5__savings_dates.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("../migrations/V6__income_card_balances.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("../migrations/V7__expense_cards.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("../migrations/V8__currencies.sql"),
    },
    Migration {
        version: 9,
        sql: include_str!("../migrations/V9__savings_currency.sql"),
    },
    Migration {
        version: 10,
        sql: include_str!("../migrations/V10__expense_card_balances.sql"),
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
    #[error("Структура базы данных не распознана; данные не изменены")]
    UnrecognizedSchema,
    #[error("История миграций повреждена; данные не изменены")]
    InvalidMigrationHistory,
    #[error("Файл базы данных не прошёл проверку целостности: {0}")]
    InvalidDatabaseIntegrity(String),
    #[error("Выбранный файл не содержит базы данных FinanceApp")]
    InvalidDatabaseSnapshot,
    #[error("Файл резервной копии уже существует; выберите другое имя")]
    BackupDestinationExists,
    #[error("Не удалось подготовить импорт: рядом с базой уже есть незавершённое восстановление")]
    ImportRecoveryInProgress,
    #[error("База данных изменилась во время подготовки обновления; повторите запуск")]
    ConcurrentDatabaseModification,
    #[error("Проверка внешних ключей после миграции не пройдена: {0}")]
    ForeignKeyIntegrity(String),
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
    #[error("Банк не найден")]
    SavingsBankUnavailable,
    #[error("Банк платёжной карты не найден")]
    PaymentDocumentBankUnavailable,
    #[error("Одно или несколько накоплений не найдены")]
    PaymentDocumentSavingsUnavailable,
    #[error("Банк платёжной карты и выбранных накоплений должен совпадать")]
    PaymentDocumentSavingsBankMismatch,
    #[error("Начало периода не может быть позже его окончания")]
    InvalidAnalysisDateRange,
    #[error("Начало периода не может быть позже его окончания")]
    InvalidTransactionDateRange,
    #[error("Выберите дебетовую карту с валютой, совпадающей с валютой дохода")]
    IncomeCardUnavailable,
    #[error("Валюта расхода должна совпадать с валютой карты; для учёта расхода карта также должна влиять на ставку")]
    ExpenseCardUnavailable,
    #[error("Недостаточно средств на дебетовой карте для операции")]
    InsufficientCardBalance,
    #[error("Не удалось точно рассчитать баланс карты: переполнение или недостаточная точность")]
    CardBalanceOverflow,
    #[error("Карта связана с доходами: нельзя удалить её или изменить её тип и валюту")]
    IncomeCardInUse,
    #[error("Карта связана с расходами: нельзя удалить её, изменить её валюту или отключить влияние на ставку")]
    ExpenseCardInUse,
    #[error("Нельзя изменить тип дебетовой карты на кредитную: с её баланса уже списаны расходы")]
    AppliedExpenseCardType,
    #[error("Валюта не найдена")]
    CurrencyNotFound,
    #[error("Основную валюту нельзя удалить; сначала выберите другую основную валюту")]
    PrimaryCurrencyCannotDelete,
    #[error("Валюта используется доходами, расходами или платёжными картами")]
    CurrencyInUse,
    #[error("Валюта с таким кодом уже существует")]
    CurrencyAlreadyExists,
    #[error("Для выбранной основной валюты не задан курс")]
    PrimaryCurrencyRateUnavailable,
    #[error("Для основной валюты нельзя задать курс относительно самой себя")]
    PrimaryCurrencyRateImmutable,
    #[error("В базе данных должна быть ровно одна основная валюта")]
    InvalidPrimaryCurrencyConfiguration,
    #[error(transparent)]
    Domain(#[from] finance_core::DomainError),
}

/// Opens the database, enables foreign keys, and applies supported migrations automatically.
/// Before upgrading a recognized older schema, creates an adjacent consistent SQLite backup.
/// An unrecognized non-empty database is rejected without modifying its contents.
pub fn initialize(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    apply_migrations(&mut connection, path)?;

    Ok(connection)
}

/// Reads an application setting from the existing settings table.
pub fn get_app_setting(connection: &Connection, key: &str) -> Result<Option<String>, DbError> {
    Ok(connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

/// Stores an application setting without changing the database schema.
pub fn set_app_setting(connection: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    connection.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Exports the whole database as a consistent SQLite snapshot without replacing an existing file.
pub fn export_database_backup(connection: &Connection, destination: &Path) -> Result<(), DbError> {
    if destination.exists() {
        return Err(DbError::BackupDestinationExists);
    }

    let temporary_path = create_temporary_file_near(destination, "export")?;
    let result = (|| {
        create_consistent_copy(connection, &temporary_path)?;
        validate_database(&temporary_path)?;
        match fs::hard_link(&temporary_path, destination) {
            Ok(()) => {
                fs::remove_file(&temporary_path)?;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(DbError::BackupDestinationExists)
            }
            Err(error) => Err(error.into()),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

/// Opens the live database read-only and exports a consistent SQLite snapshot.
pub fn export_database_backup_from_path(
    database_path: &Path,
    destination: &Path,
) -> Result<(), DbError> {
    let connection = Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    export_database_backup(&connection, destination)
}

/// Validates and stages a complete SQLite backup for import after the app closes its connections.
pub fn stage_database_import(source: &Path, database_path: &Path) -> Result<(), DbError> {
    let pending_path = pending_import_path(database_path);
    if pending_path.exists() {
        return Err(DbError::ImportRecoveryInProgress);
    }

    let source_connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source_connection.busy_timeout(Duration::from_secs(5))?;
    validate_database_connection(&source_connection)?;

    let work_directory = create_import_work_directory(database_path)?;
    let candidate_path = work_directory.join("candidate.db");
    let result = (|| {
        create_consistent_copy(&source_connection, &candidate_path)?;
        validate_database_snapshot(&candidate_path)?;

        let migrated_candidate = initialize(&candidate_path)?;
        drop(migrated_candidate);
        validate_database(&candidate_path)?;

        for entry in fs::read_dir(&work_directory)? {
            let entry = entry?;
            if entry.path() != candidate_path {
                fs::remove_file(entry.path())?;
            }
        }

        fs::rename(&candidate_path, &pending_path)?;
        if let Err(error) = fs::remove_dir(&work_directory) {
            let _ = fs::remove_file(&pending_path);
            return Err(error.into());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&work_directory);
    }
    result
}

/// Reports whether a staged import is waiting for the application to close.
pub fn has_pending_database_import(database_path: &Path) -> bool {
    pending_import_path(database_path).exists()
}

/// Installs a previously validated snapshot, retaining a consistent backup of the current database.
pub fn restore_pending_database_import(database_path: &Path) -> Result<bool, DbError> {
    recover_interrupted_import(database_path)?;

    let pending_path = pending_import_path(database_path);
    if !pending_path.exists() {
        return Ok(false);
    }
    validate_database(&pending_path)?;

    let rollback_path = rollback_import_path(database_path);
    if rollback_path.exists() {
        return Err(DbError::ImportRecoveryInProgress);
    }

    if database_path.exists() {
        let current_database =
            Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let backup_path = unique_sibling_path(database_path, "before-import")?;
        export_database_backup(&current_database, &backup_path)?;
        drop(current_database);
        fs::rename(database_path, &rollback_path)?;
    }

    if let Err(error) = fs::rename(&pending_path, database_path) {
        if rollback_path.exists() {
            fs::rename(&rollback_path, database_path)?;
        }
        return Err(error.into());
    }

    if rollback_path.exists() {
        fs::remove_file(rollback_path)?;
    }
    Ok(true)
}

fn create_consistent_copy(source: &Connection, destination: &Path) -> Result<(), DbError> {
    let mut target = Connection::open(destination)?;
    target.busy_timeout(Duration::from_secs(5))?;
    {
        let backup = Backup::new(source, &mut target)?;
        backup.run_to_completion(128, Duration::from_millis(10), None)?;
    }
    validate_database_connection(&target)?;
    Ok(())
}

fn validate_database(path: &Path) -> Result<(), DbError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    validate_database_connection(&connection)?;
    validate_current_schema(&connection)
}

fn validate_database_snapshot(path: &Path) -> Result<(), DbError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    validate_database_connection(&connection)
}

fn validate_database_connection(connection: &Connection) -> Result<(), DbError> {
    let integrity =
        connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
    if integrity != "ok" {
        return Err(DbError::InvalidDatabaseIntegrity(integrity));
    }

    let foreign_key_violation =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })?;
    if foreign_key_violation > 0 {
        return Err(DbError::ForeignKeyIntegrity(
            "обнаружены нарушения ссылочной целостности".to_owned(),
        ));
    }

    if detect_migration_version(connection)?.is_none() {
        return Err(DbError::InvalidDatabaseSnapshot);
    }
    Ok(())
}

fn validate_current_schema(connection: &Connection) -> Result<(), DbError> {
    let current_version = MIGRATIONS.last().map_or(0, |migration| migration.version);
    if detect_migration_version(connection)? != Some(current_version) {
        return Err(DbError::InvalidDatabaseSnapshot);
    }

    for table in [
        "app_settings",
        "banks",
        "income_categories",
        "expense_categories",
        "incomes",
        "expenses",
        "savings",
        "payment_documents",
        "document_rate_links",
        "currencies",
    ] {
        let exists = connection.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
            )",
            [table],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(DbError::InvalidDatabaseSnapshot);
        }
    }
    Ok(())
}

fn create_temporary_file_near(path: &Path, purpose: &str) -> Result<PathBuf, DbError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("database.db");

    for attempt in 0..100 {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let temporary_path = parent.join(format!(
            ".{file_name}.{purpose}-{}-{timestamp}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(_) => return Ok(temporary_path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "не удалось создать временный файл для операции с базой данных",
    )
    .into())
}

fn create_import_work_directory(database_path: &Path) -> Result<PathBuf, DbError> {
    let parent = database_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for attempt in 0..100 {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let work_directory = parent.join(format!(
            ".financeapp-import-{}-{timestamp}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&work_directory) {
            Ok(()) => return Ok(work_directory),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "не удалось создать временный каталог для импорта",
    )
    .into())
}

fn pending_import_path(database_path: &Path) -> PathBuf {
    let mut file_name = database_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("data.db"))
        .to_os_string();
    file_name.push(".import-pending");
    database_path.with_file_name(file_name)
}

fn rollback_import_path(database_path: &Path) -> PathBuf {
    let mut file_name = database_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("data.db"))
        .to_os_string();
    file_name.push(".import-rollback");
    database_path.with_file_name(file_name)
}

fn recover_interrupted_import(database_path: &Path) -> Result<(), DbError> {
    let rollback_path = rollback_import_path(database_path);
    if !rollback_path.exists() {
        return Ok(());
    }

    if !database_path.exists() {
        fs::rename(rollback_path, database_path)?;
        return Ok(());
    }

    let recovered_path = unique_sibling_path(database_path, "recovered-import")?;
    fs::rename(rollback_path, recovered_path)?;
    Ok(())
}

fn unique_sibling_path(path: &Path, label: &str) -> Result<PathBuf, DbError> {
    let file_stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("database");
    for attempt in 0..100 {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let candidate =
            path.with_file_name(format!("{file_stem}.{label}-{timestamp}-{attempt}.db"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "не удалось выбрать свободное имя файла резервной копии",
    )
    .into())
}

/// Returns the configured currencies, including the primary currency and known rates.
pub fn list_currencies(connection: &Connection) -> Result<Vec<CurrencyDefinition>, DbError> {
    let mut statement = connection.prepare(
        "SELECT code, name, is_primary, rate_to_primary, rate_date
         FROM currencies
         ORDER BY is_primary DESC, code",
    )?;
    let currencies = statement
        .query_map([], |row| {
            let rate_text = row.get::<_, Option<String>>(3)?;
            let rate_to_primary = rate_text
                .map(|rate| {
                    Decimal::from_str(&rate).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
                    })
                })
                .transpose()?;
            let date_text = row.get::<_, Option<String>>(4)?;
            let rate_date = date_text
                .map(|date| {
                    NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
                    })
                })
                .transpose()?;
            Ok(CurrencyDefinition {
                code: row.get(0)?,
                name: row.get(1)?,
                is_primary: row.get(2)?,
                rate_to_primary,
                rate_date,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(currencies)
}

/// Lists currencies referenced by income, expense, or payment-document records.
pub fn list_used_currency_codes(connection: &Connection) -> Result<Vec<String>, DbError> {
    let mut statement = connection.prepare(
        "SELECT currency FROM incomes
         UNION SELECT currency FROM expenses
         UNION SELECT currency FROM payment_documents WHERE currency IS NOT NULL
         ORDER BY currency",
    )?;
    let currencies = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(currencies)
}

/// Adds a currency with no known rate relative to the current primary currency.
pub fn add_currency(connection: &Connection, code: &str, name: &str) -> Result<(), DbError> {
    validate_currency_code(code)?;
    if name.trim().is_empty() {
        return Err(DbError::EmptyName);
    }
    match connection.execute(
        "INSERT INTO currencies (code, name) VALUES (?1, ?2)",
        params![code, name.trim()],
    ) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Err(DbError::CurrencyAlreadyExists);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Deletes a currency only when it is neither primary nor referenced by financial records.
pub fn delete_currency(connection: &Connection, code: &str) -> Result<(), DbError> {
    validate_currency_code(code)?;
    let is_primary = connection
        .query_row(
            "SELECT is_primary FROM currencies WHERE code = ?1",
            [code],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .ok_or(DbError::CurrencyNotFound)?;
    if is_primary {
        return Err(DbError::PrimaryCurrencyCannotDelete);
    }

    let is_in_use = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM incomes WHERE currency = ?1)
             OR EXISTS(SELECT 1 FROM expenses WHERE currency = ?1)
             OR EXISTS(SELECT 1 FROM payment_documents WHERE currency = ?1)",
        [code],
        |row| row.get::<_, bool>(0),
    )?;
    if is_in_use {
        return Err(DbError::CurrencyInUse);
    }

    let deleted = match connection.execute("DELETE FROM currencies WHERE code = ?1", [code]) {
        Ok(deleted) => deleted,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Err(DbError::CurrencyInUse);
        }
        Err(error) => return Err(error.into()),
    };
    if deleted == 0 {
        return Err(DbError::CurrencyNotFound);
    }
    Ok(())
}

/// Sets a non-primary currency's positive rate and the date the rate applies to.
pub fn set_currency_exchange_rate(
    connection: &Connection,
    code: &str,
    rate: Decimal,
    rate_date: NaiveDate,
) -> Result<(), DbError> {
    validate_currency_code(code)?;
    validate_exchange_rate(rate)?;
    let is_primary = connection
        .query_row(
            "SELECT is_primary FROM currencies WHERE code = ?1",
            [code],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .ok_or(DbError::CurrencyNotFound)?;
    if is_primary {
        return Err(DbError::PrimaryCurrencyRateImmutable);
    }
    connection.execute(
        "UPDATE currencies SET rate_to_primary = ?1, rate_date = ?2 WHERE code = ?3",
        params![
            canonical_amount(rate),
            rate_date.format("%Y-%m-%d").to_string(),
            code
        ],
    )?;
    Ok(())
}

/// Changes the primary currency and rebases all known rates atomically.
///
/// The target currency must have a known rate. Currencies without a known rate remain
/// unknown; transaction amounts are not modified.
pub fn set_primary_currency(connection: &mut Connection, code: &str) -> Result<(), DbError> {
    validate_currency_code(code)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let currencies = list_currencies(&transaction)?;
    if currencies
        .iter()
        .filter(|currency| currency.is_primary)
        .count()
        != 1
    {
        return Err(DbError::InvalidPrimaryCurrencyConfiguration);
    }
    let target = currencies
        .iter()
        .find(|currency| currency.code == code)
        .ok_or(DbError::CurrencyNotFound)?;
    if target.is_primary {
        transaction.commit()?;
        return Ok(());
    }
    let new_primary_rate = target
        .rate_to_primary
        .ok_or(DbError::PrimaryCurrencyRateUnavailable)?;
    let new_primary_rate_date = target
        .rate_date
        .ok_or(DbError::PrimaryCurrencyRateUnavailable)?;

    let mut rebased_currencies = Vec::with_capacity(currencies.len());
    for currency in &currencies {
        if currency.code == code {
            continue;
        }
        let Some(old_rate) = currency
            .rate_to_primary
            .or_else(|| currency.is_primary.then_some(Decimal::ONE))
        else {
            rebased_currencies.push((currency.code.as_str(), None, None));
            continue;
        };
        let rate = rebase_exchange_rate(old_rate, new_primary_rate)?;
        let rate_date = currency.rate_date.map_or(new_primary_rate_date, |date| {
            date.min(new_primary_rate_date)
        });
        rebased_currencies.push((currency.code.as_str(), Some(rate), Some(rate_date)));
    }

    for (currency_code, rate, rate_date) in rebased_currencies {
        let changed = transaction.execute(
            "UPDATE currencies
             SET is_primary = 0, rate_to_primary = ?1, rate_date = ?2
             WHERE code = ?3",
            params![
                rate.map(canonical_amount),
                rate_date.map(|date| date.format("%Y-%m-%d").to_string()),
                currency_code
            ],
        )?;
        if changed != 1 {
            return Err(DbError::CurrencyNotFound);
        }
    }
    let changed = transaction.execute(
        "UPDATE currencies
         SET is_primary = 1, rate_to_primary = NULL, rate_date = NULL
         WHERE code = ?1",
        [code],
    )?;
    if changed != 1 {
        return Err(DbError::CurrencyNotFound);
    }
    transaction.commit()?;
    Ok(())
}

/// Returns transactions of one type, optionally filtered by inclusive dates, category, currency, and text.
pub fn list_transactions(
    connection: &Connection,
    kind: TransactionKind,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    category_id: Option<i64>,
    currency: Option<Currency>,
    search: &str,
) -> Result<Vec<FinancialTransaction>, DbError> {
    let (table, categories) = transaction_tables(kind);
    let (card_column, rate_effect_column, balance_applied_column) = match kind {
        TransactionKind::Income => ("t.payment_document_id", "0", "0"),
        TransactionKind::Expense => (
            "t.payment_document_id",
            "t.affects_deposit_rate",
            "t.payment_document_balance_applied",
        ),
    };
    let sql = format!(
        "SELECT t.id, t.date, t.category_id, c.name, t.amount, t.currency, t.comment,
                {card_column}, {rate_effect_column}, {balance_applied_column}
         FROM {table} t JOIN {categories} c ON c.id = t.category_id
         WHERE (?1 IS NULL OR t.date >= ?1)
           AND (?2 IS NULL OR t.date <= ?2)
           AND (?3 IS NULL OR t.category_id = ?3)
           AND (?4 IS NULL OR t.currency = ?4)
         ORDER BY t.date DESC, t.id DESC"
    );
    let from = from.map(|date| date.format("%Y-%m-%d").to_string());
    let to = to.map(|date| date.format("%Y-%m-%d").to_string());
    let currency = currency.map(Currency::code);
    let mut statement = connection.prepare(&sql)?;
    let transactions = statement
        .query_map(params![from, to, category_id, currency], |row| {
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
                payment_document_id: row.get(7)?,
                affects_deposit_rate: row.get(8)?,
                payment_document_balance_applied: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let search = search.trim().to_lowercase();
    if search.is_empty() {
        Ok(transactions)
    } else {
        Ok(transactions
            .into_iter()
            .filter(|transaction| {
                transaction.comment.to_lowercase().contains(&search)
                    || transaction.category_name.to_lowercase().contains(&search)
            })
            .collect())
    }
}

/// Reads income and expense amounts for an inclusive date range, without category or text filters.
pub fn list_transaction_amounts(
    connection: &Connection,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<Vec<TransactionAmount>, DbError> {
    if matches!((from, to), (Some(from), Some(to)) if from > to) {
        return Err(DbError::InvalidTransactionDateRange);
    }

    let from = from.map(|date| date.format("%Y-%m-%d").to_string());
    let to = to.map(|date| date.format("%Y-%m-%d").to_string());
    let mut statement = connection.prepare(
        "SELECT kind, amount, currency
         FROM (
             SELECT 'income' AS kind, amount, currency, date FROM incomes
             UNION ALL
             SELECT 'expense' AS kind, amount, currency, date FROM expenses
         )
         WHERE (?1 IS NULL OR date >= ?1)
           AND (?2 IS NULL OR date <= ?2)",
    )?;
    let transactions = statement
        .query_map(params![from, to], |row| {
            let kind_text = row.get::<_, String>(0)?;
            let kind = match kind_text.as_str() {
                "income" => TransactionKind::Income,
                "expense" => TransactionKind::Expense,
                _ => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        0,
                        Type::Text,
                        Box::new(std::io::Error::other("Неизвестный тип финансовой операции")),
                    ));
                }
            };
            let amount_text = row.get::<_, String>(1)?;
            let amount = Decimal::from_str(&amount_text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(1, Type::Text, Box::new(error))
            })?;
            let currency_text = row.get::<_, String>(2)?;
            let currency = Currency::from_str(&currency_text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
            })?;
            Ok(TransactionAmount {
                kind,
                amount,
                currency,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

/// Reads report transactions through a read-only connection and one snapshot transaction.
pub fn load_analysis_transactions(
    database_path: &Path,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    kind_filter: Option<TransactionKind>,
) -> Result<Vec<AnalysisTransaction>, DbError> {
    load_analysis_transactions_with_filters(database_path, from, to, kind_filter, None, None)
}

/// Reads report transactions filtered independently by income and expense category.
pub fn load_analysis_transactions_by_category(
    database_path: &Path,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    income_category_id: Option<i64>,
    expense_category_id: Option<i64>,
) -> Result<Vec<AnalysisTransaction>, DbError> {
    load_analysis_transactions_with_filters(
        database_path,
        from,
        to,
        None,
        income_category_id,
        expense_category_id,
    )
}

fn load_analysis_transactions_with_filters(
    database_path: &Path,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    kind_filter: Option<TransactionKind>,
    income_category_id: Option<i64>,
    expense_category_id: Option<i64>,
) -> Result<Vec<AnalysisTransaction>, DbError> {
    if matches!((from, to), (Some(from), Some(to)) if from > to) {
        return Err(DbError::InvalidAnalysisDateRange);
    }

    let mut connection =
        Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let from = from.map(|date| date.format("%Y-%m-%d").to_string());
    let to = to.map(|date| date.format("%Y-%m-%d").to_string());
    let mut entries = Vec::new();

    for kind in [TransactionKind::Income, TransactionKind::Expense] {
        if kind_filter.is_some_and(|filter| filter != kind) {
            continue;
        }

        let (table, categories) = transaction_tables(kind);
        let category_id_filter = match kind {
            TransactionKind::Income => income_category_id,
            TransactionKind::Expense => expense_category_id,
        };
        let sql = format!(
            "SELECT t.category_id, c.name, t.amount, t.currency
             FROM {table} t JOIN {categories} c ON c.id = t.category_id
             WHERE (?1 IS NULL OR t.date >= ?1)
               AND (?2 IS NULL OR t.date <= ?2)
               AND (?3 IS NULL OR t.category_id = ?3)
             ORDER BY t.date, t.id"
        );
        let mut statement = transaction.prepare(&sql)?;
        let rows = statement.query_map(params![from, to, category_id_filter], |row| {
            let amount_text = row.get::<_, String>(2)?;
            let amount = Decimal::from_str(&amount_text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
            })?;
            let currency_text = row.get::<_, String>(3)?;
            let currency = Currency::from_str(&currency_text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
            })?;
            Ok(AnalysisTransaction {
                kind,
                category_id: row.get(0)?,
                category_name: row.get(1)?,
                amount,
                currency,
            })
        })?;
        entries.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }

    transaction.commit()?;
    Ok(entries)
}

/// Creates or updates an income or expense, retaining the decimal as canonical text.
// The existing UI and repository callers pass these fields independently.
#[expect(
    clippy::too_many_arguments,
    reason = "Preserve the established transaction API"
)]
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
    let (card, affects_deposit_rate) = match (kind, id) {
        (TransactionKind::Income, Some(id)) => (
            connection.query_row(
                "SELECT payment_document_id FROM incomes WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?,
            false,
        ),
        (TransactionKind::Expense, Some(id)) => connection.query_row(
            "SELECT payment_document_id, affects_deposit_rate FROM expenses WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?,
        (_, None) => (None, false),
    };
    save_transaction_with_card_and_rate_effect(
        connection,
        kind,
        id,
        date,
        category_id,
        amount,
        currency,
        comment,
        card,
        affects_deposit_rate,
    )
}

/// Saves a transaction with an optional payment document; rate-counted expenses require a qualifying card.
#[expect(
    clippy::too_many_arguments,
    reason = "Matches the existing transaction API"
)]
pub fn save_transaction_with_card(
    connection: &Connection,
    kind: TransactionKind,
    id: Option<i64>,
    date: NaiveDate,
    category_id: i64,
    amount: Decimal,
    currency: Currency,
    comment: &str,
    payment_document_id: Option<i64>,
) -> Result<i64, DbError> {
    save_transaction_with_card_and_rate_effect(
        connection,
        kind,
        id,
        date,
        category_id,
        amount,
        currency,
        comment,
        payment_document_id,
        false,
    )
}

/// Saves a transaction with its optional card and whether an expense counts toward deposit-rate conditions.
#[expect(
    clippy::too_many_arguments,
    reason = "Matches the established transaction API while adding the expense rate flag"
)]
pub fn save_transaction_with_card_and_rate_effect(
    connection: &Connection,
    kind: TransactionKind,
    id: Option<i64>,
    date: NaiveDate,
    category_id: i64,
    amount: Decimal,
    currency: Currency,
    comment: &str,
    payment_document_id: Option<i64>,
    affects_deposit_rate: bool,
) -> Result<i64, DbError> {
    if amount <= Decimal::ZERO {
        return Err(DbError::InvalidAmount);
    }
    if kind == TransactionKind::Income && affects_deposit_rate {
        return Err(DbError::ExpenseCardUnavailable);
    }
    if kind == TransactionKind::Expense && affects_deposit_rate && payment_document_id.is_none() {
        return Err(DbError::ExpenseCardUnavailable);
    }
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let (table, categories) = transaction_tables(kind);
    let current_category = id
        .map(|id| {
            transaction
                .query_row(
                    &format!("SELECT category_id FROM {table} WHERE id = ?1"),
                    [id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
        })
        .transpose()?
        .flatten();
    let category_is_active = transaction
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

    let old_credit = if kind == TransactionKind::Income {
        match id {
            Some(id) => {
                let (card, old_amount): (Option<i64>, String) = transaction.query_row(
                    "SELECT payment_document_id, amount FROM incomes WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                card.map(|card| parse_decimal_text(old_amount, 1).map(|amount| (card, amount)))
                    .transpose()?
            }
            None => None,
        }
    } else {
        None
    };
    let old_debit_expense = if kind == TransactionKind::Expense {
        match id {
            Some(id) => {
                let (card, old_amount, balance_applied): (Option<i64>, String, bool) = transaction
                    .query_row(
                        "SELECT payment_document_id, amount, payment_document_balance_applied
                         FROM expenses WHERE id = ?1",
                        [id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )?;
                Some((card, parse_decimal_text(old_amount, 1)?, balance_applied))
            }
            None => None,
        }
    } else {
        None
    };
    let mut payment_document_is_debit = false;
    if let Some(card_id) = payment_document_id {
        let is_debit = match kind {
            TransactionKind::Income => transaction
                .query_row(
                    "SELECT card_type = 'debit' FROM payment_documents
                     WHERE id = ?1 AND card_type = 'debit' AND currency = ?2",
                    params![card_id, currency.code()],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?,
            TransactionKind::Expense => transaction
                .query_row(
                    "SELECT card_type = 'debit' FROM payment_documents
                     WHERE id = ?1 AND currency = ?2
                       AND (?3 = 0 OR affects_deposit_rate = 1)",
                    params![card_id, currency.code(), affects_deposit_rate],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?,
        };
        payment_document_is_debit = is_debit.ok_or(match kind {
            TransactionKind::Income => DbError::IncomeCardUnavailable,
            TransactionKind::Expense => DbError::ExpenseCardUnavailable,
        })?;
    }
    if kind == TransactionKind::Income {
        if let Some((old_card, old_amount)) = old_credit {
            if Some(old_card) == payment_document_id {
                let delta = amount
                    .checked_sub(old_amount)
                    .ok_or(DbError::CardBalanceOverflow)?;
                if delta.checked_add(old_amount) != Some(amount)
                    || amount.checked_sub(delta) != Some(old_amount)
                {
                    return Err(DbError::CardBalanceOverflow);
                }
                adjust_card_balance(&transaction, old_card, delta)?;
            } else {
                adjust_card_balance(&transaction, old_card, -old_amount)?;
                if let Some(new_card) = payment_document_id {
                    adjust_card_balance(&transaction, new_card, amount)?;
                }
            }
        } else if let Some(new_card) = payment_document_id {
            adjust_card_balance(&transaction, new_card, amount)?;
        }
    } else {
        match old_debit_expense {
            Some((Some(old_card), old_amount, true)) => {
                if Some(old_card) == payment_document_id && payment_document_is_debit {
                    let delta = old_amount
                        .checked_sub(amount)
                        .ok_or(DbError::CardBalanceOverflow)?;
                    if delta.checked_add(amount) != Some(old_amount)
                        || old_amount.checked_sub(delta) != Some(amount)
                    {
                        return Err(DbError::CardBalanceOverflow);
                    }
                    adjust_card_balance(&transaction, old_card, delta)?;
                } else {
                    adjust_card_balance(&transaction, old_card, old_amount)?;
                    if payment_document_is_debit {
                        let debit = Decimal::ZERO
                            .checked_sub(amount)
                            .ok_or(DbError::CardBalanceOverflow)?;
                        adjust_card_balance(
                            &transaction,
                            payment_document_id.ok_or(DbError::ExpenseCardUnavailable)?,
                            debit,
                        )?;
                    }
                }
            }
            Some((None, _, true)) => return Err(DbError::ExpenseCardUnavailable),
            Some((_, _, false)) | None if payment_document_is_debit => {
                let debit = Decimal::ZERO
                    .checked_sub(amount)
                    .ok_or(DbError::CardBalanceOverflow)?;
                adjust_card_balance(
                    &transaction,
                    payment_document_id.ok_or(DbError::ExpenseCardUnavailable)?,
                    debit,
                )?;
            }
            Some((_, _, false)) | None => {}
        }
    }
    let amount = canonical_amount(amount);
    let date = date.format("%Y-%m-%d").to_string();
    let saved_id = if let Some(id) = id {
        let changed = if kind == TransactionKind::Income {
            transaction.execute(
                "UPDATE incomes SET date = ?1, category_id = ?2, amount = ?3,
                 currency = ?4, comment = ?5, payment_document_id = ?6 WHERE id = ?7",
                params![
                    date,
                    category_id,
                    amount,
                    currency.code(),
                    comment,
                    payment_document_id,
                    id
                ],
            )?
        } else {
            transaction.execute(
                &format!(
                    "UPDATE {table} SET date = ?1, category_id = ?2, amount = ?3,
                 currency = ?4, comment = ?5, payment_document_id = ?6,
                 affects_deposit_rate = ?7, payment_document_balance_applied = ?8 WHERE id = ?9"
                ),
                params![
                    date,
                    category_id,
                    amount,
                    currency.code(),
                    comment,
                    payment_document_id,
                    affects_deposit_rate,
                    payment_document_is_debit,
                    id
                ],
            )?
        };
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        id
    } else {
        if kind == TransactionKind::Income {
            transaction.execute(
                "INSERT INTO incomes (date, category_id, amount, currency, comment)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![date, category_id, amount, currency.code(), comment],
            )?;
        } else {
            transaction.execute(
                "INSERT INTO expenses (
                    date, category_id, amount, currency, comment,
                    payment_document_id, affects_deposit_rate, payment_document_balance_applied
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    date,
                    category_id,
                    amount,
                    currency.code(),
                    comment,
                    payment_document_id,
                    affects_deposit_rate,
                    payment_document_is_debit
                ],
            )?;
        }
        transaction.last_insert_rowid()
    };
    if kind == TransactionKind::Income && id.is_none() {
        transaction.execute(
            "UPDATE incomes SET payment_document_id = ?1 WHERE id = ?2",
            params![payment_document_id, saved_id],
        )?;
    }
    transaction.commit()?;
    Ok(saved_id)
}

fn adjust_card_balance(connection: &Connection, id: i64, delta: Decimal) -> Result<(), DbError> {
    let text: String = connection.query_row(
        "SELECT amount FROM payment_documents WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    let balance = parse_decimal_text(text, 0)?;
    let updated = balance
        .checked_add(delta)
        .ok_or(DbError::CardBalanceOverflow)?;
    if updated.checked_sub(delta) != Some(balance) || updated.checked_sub(balance) != Some(delta) {
        return Err(DbError::CardBalanceOverflow);
    }
    if updated < Decimal::ZERO {
        return Err(DbError::InsufficientCardBalance);
    }
    connection.execute(
        "UPDATE payment_documents SET amount = ?1 WHERE id = ?2",
        params![canonical_amount(updated), id],
    )?;
    Ok(())
}

/// Deletes a transaction by its database identifier.
pub fn delete_transaction(
    connection: &Connection,
    kind: TransactionKind,
    id: i64,
) -> Result<(), DbError> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    if kind == TransactionKind::Income {
        let (card, amount): (Option<i64>, String) = transaction.query_row(
            "SELECT payment_document_id, amount FROM incomes WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if let Some(card) = card {
            adjust_card_balance(&transaction, card, -parse_decimal_text(amount, 1)?)?;
        }
    } else {
        let (card, amount, balance_applied): (Option<i64>, String, bool) = transaction.query_row(
            "SELECT payment_document_id, amount, payment_document_balance_applied
                 FROM expenses WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if balance_applied {
            let card = card.ok_or(DbError::ExpenseCardUnavailable)?;
            adjust_card_balance(&transaction, card, parse_decimal_text(amount, 1)?)?;
        }
    }
    let (table, _) = transaction_tables(kind);
    let deleted = transaction.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
    if deleted == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows.into());
    }
    transaction.commit()?;
    Ok(())
}

/// Lists savings accounts with optional bank, activity, and case-insensitive text filters.
pub fn list_savings(
    connection: &Connection,
    bank_id: Option<i64>,
    is_active: Option<bool>,
    search: &str,
) -> Result<Vec<SavingsAccount>, DbError> {
    list_savings_filtered(connection, bank_id, is_active, None, search)
}

/// Lists savings accounts with optional bank, activity, currency, and text filters.
pub fn list_savings_filtered(
    connection: &Connection,
    bank_id: Option<i64>,
    is_active: Option<bool>,
    currency: Option<Currency>,
    search: &str,
) -> Result<Vec<SavingsAccount>, DbError> {
    let mut statement = connection.prepare(
        "SELECT s.id, s.bank_id, b.name, s.title, s.amount, s.base_rate,
                s.rate_after_threshold, s.threshold_amount, s.term_type, s.term_end_date,
                s.interest_destination, s.is_active, s.opened_date, s.interest_accrual_date,
                s.currency
         FROM savings s JOIN banks b ON b.id = s.bank_id
         WHERE (?1 IS NULL OR s.bank_id = ?1)
           AND (?2 IS NULL OR s.is_active = ?2)
           AND (?3 IS NULL OR s.currency = ?3)
         ORDER BY b.name COLLATE NOCASE, s.title COLLATE NOCASE, s.id",
    )?;
    let rows = statement.query_map(
        params![bank_id, is_active, currency.map(Currency::code)],
        |row| {
            let amount = parse_decimal_text(row.get(4)?, 4)?;
            let base_rate = parse_decimal_text(row.get(5)?, 5)?;
            let rate_after_threshold = row
                .get::<_, Option<String>>(6)?
                .map(|value| parse_decimal_text(value, 6))
                .transpose()?;
            let threshold_amount = row
                .get::<_, Option<String>>(7)?
                .map(|value| parse_decimal_text(value, 7))
                .transpose()?;
            let term_type_text = row.get::<_, String>(8)?;
            let term_type = term_type_text.parse::<SavingsTerm>().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
            })?;
            let term_end_date = row
                .get::<_, Option<String>>(9)?
                .map(|value| {
                    NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(9, Type::Text, Box::new(error))
                    })
                })
                .transpose()?;
            let interest_destination_text = row.get::<_, String>(10)?;
            let interest_destination = interest_destination_text
                .parse::<InterestDestination>()
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(10, Type::Text, Box::new(error))
                })?;
            let opened_date = row
                .get::<_, Option<String>>(12)?
                .map(|value| {
                    NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(12, Type::Text, Box::new(error))
                    })
                })
                .transpose()?;
            let interest_accrual_date = row
                .get::<_, Option<String>>(13)?
                .map(|value| {
                    NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(13, Type::Text, Box::new(error))
                    })
                })
                .transpose()?;

            Ok(SavingsAccount {
                id: row.get(0)?,
                bank_id: row.get(1)?,
                bank_name: row.get(2)?,
                title: row.get(3)?,
                amount,
                currency: row
                    .get::<_, Option<String>>(14)?
                    .map(|value| {
                        value.parse().map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                14,
                                Type::Text,
                                Box::new(error),
                            )
                        })
                    })
                    .transpose()?,
                base_rate,
                rate_after_threshold,
                threshold_amount,
                opened_date,
                interest_accrual_date,
                term_type,
                term_end_date,
                interest_destination,
                is_active: row.get(11)?,
            })
        },
    )?;
    let accounts = rows.collect::<Result<Vec<_>, _>>()?;
    let search = search.trim().to_lowercase();
    if search.is_empty() {
        Ok(accounts)
    } else {
        Ok(accounts
            .into_iter()
            .filter(|account| {
                account.title.to_lowercase().contains(&search)
                    || account.bank_name.to_lowercase().contains(&search)
            })
            .collect())
    }
}

/// Creates or updates a savings account after validating its domain fields and bank.
pub fn save_savings(
    connection: &Connection,
    id: Option<i64>,
    savings: &SavingsDetails,
) -> Result<i64, DbError> {
    savings.validate()?;
    let bank_exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM banks WHERE id = ?1)",
        [savings.bank_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !bank_exists {
        return Err(DbError::SavingsBankUnavailable);
    }

    let amount = canonical_amount(savings.amount);
    let base_rate = canonical_amount(savings.base_rate);
    let rate_after_threshold = savings.rate_after_threshold.map(canonical_amount);
    let threshold_amount = savings.threshold_amount.map(canonical_amount);
    let term_end_date = savings
        .term_end_date
        .map(|date| date.format("%Y-%m-%d").to_string());
    let opened_date = savings
        .opened_date
        .map(|date| date.format("%Y-%m-%d").to_string());
    let interest_accrual_date = savings
        .interest_accrual_date
        .map(|date| date.format("%Y-%m-%d").to_string());
    let currency = savings.currency.map(Currency::code);
    if let Some(id) = id {
        let changed = connection.execute(
            "UPDATE savings
             SET bank_id = ?1, title = ?2, amount = ?3, base_rate = ?4,
                 rate_after_threshold = ?5, threshold_amount = ?6, term_type = ?7,
                 term_end_date = ?8, opened_date = ?9, interest_accrual_date = ?10,
                 interest_destination = ?11, is_active = ?12, currency = ?13
             WHERE id = ?14",
            params![
                savings.bank_id,
                savings.title.trim(),
                amount,
                base_rate,
                rate_after_threshold,
                threshold_amount,
                savings.term_type.code(),
                term_end_date,
                opened_date,
                interest_accrual_date,
                savings.interest_destination.code(),
                savings.is_active,
                currency,
                id
            ],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        Ok(id)
    } else {
        connection.execute(
            "INSERT INTO savings (
                bank_id, title, amount, base_rate, rate_after_threshold, threshold_amount,
                term_type, term_end_date, opened_date, interest_accrual_date,
                interest_destination, is_active, currency
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                savings.bank_id,
                savings.title.trim(),
                amount,
                base_rate,
                rate_after_threshold,
                threshold_amount,
                savings.term_type.code(),
                term_end_date,
                opened_date,
                interest_accrual_date,
                savings.interest_destination.code(),
                savings.is_active,
                currency
            ],
        )?;
        Ok(connection.last_insert_rowid())
    }
}

/// Deletes a savings account without cascading into other financial records.
pub fn delete_savings(connection: &Connection, id: i64) -> Result<(), DbError> {
    let deleted = connection.execute("DELETE FROM savings WHERE id = ?1", [id])?;
    if deleted == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows.into());
    }
    Ok(())
}

/// Lists payment documents filtered by bank, card type, and payment system.
pub fn list_payment_documents(
    connection: &Connection,
    bank_id: Option<i64>,
    card_type: Option<PaymentCardType>,
    payment_system: Option<PaymentSystem>,
) -> Result<Vec<PaymentDocument>, DbError> {
    list_payment_documents_filtered(connection, bank_id, card_type, payment_system, None)
}

/// Lists payment documents filtered by bank, card type, payment system, and currency.
pub fn list_payment_documents_filtered(
    connection: &Connection,
    bank_id: Option<i64>,
    card_type: Option<PaymentCardType>,
    payment_system: Option<PaymentSystem>,
    currency: Option<Currency>,
) -> Result<Vec<PaymentDocument>, DbError> {
    let mut statement = connection.prepare(
        "SELECT document.id, document.bank_id, bank.name, document.payment_system,
                document.card_type, document.amount, document.due_date,
                document.grace_period_days, document.affects_deposit_rate,
                document.min_monthly_spend, document.currency
         FROM payment_documents AS document
         JOIN banks AS bank ON bank.id = document.bank_id
         WHERE (?1 IS NULL OR document.bank_id = ?1)
           AND (?2 IS NULL OR document.card_type = ?2)
           AND (?3 IS NULL OR document.payment_system = ?3)
           AND (?4 IS NULL OR document.currency = ?4)
         ORDER BY bank.name COLLATE NOCASE, document.payment_system,
                  document.card_type, document.id",
    )?;
    let mut documents = statement
        .query_map(
            params![
                bank_id,
                card_type.map(PaymentCardType::code),
                payment_system.map(PaymentSystem::code),
                currency.map(Currency::code)
            ],
            |row| {
                let payment_system_text = row.get::<_, String>(3)?;
                let payment_system = payment_system_text.parse().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
                })?;
                let card_type_text = row.get::<_, String>(4)?;
                let card_type = card_type_text.parse().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
                })?;
                let due_date = row
                    .get::<_, Option<String>>(6)?
                    .map(|value| {
                        NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                6,
                                Type::Text,
                                Box::new(error),
                            )
                        })
                    })
                    .transpose()?;

                Ok(PaymentDocument {
                    id: row.get(0)?,
                    bank_id: row.get(1)?,
                    bank_name: row.get(2)?,
                    payment_system,
                    card_type,
                    amount: parse_decimal_text(row.get(5)?, 5)?,
                    currency: row
                        .get::<_, Option<String>>(10)?
                        .map(|value| {
                            value.parse().map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    10,
                                    Type::Text,
                                    Box::new(error),
                                )
                            })
                        })
                        .transpose()?,
                    due_date,
                    grace_period_days: row.get(7)?,
                    affects_deposit_rate: row.get(8)?,
                    min_monthly_spend: row
                        .get::<_, Option<String>>(9)?
                        .map(|value| parse_decimal_text(value, 9))
                        .transpose()?,
                    linked_savings: Vec::new(),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;

    let document_indexes: HashMap<_, _> = documents
        .iter()
        .enumerate()
        .map(|(index, document)| (document.id, index))
        .collect();
    let mut statement = connection.prepare(
        "SELECT link.document_id, savings.id, savings.title, savings.is_active
         FROM document_rate_links AS link
         JOIN savings ON savings.id = link.savings_id
         ORDER BY savings.title COLLATE NOCASE, savings.id",
    )?;
    let linked_savings = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                LinkedSavingsAccount {
                    id: row.get(1)?,
                    title: row.get(2)?,
                    is_active: row.get(3)?,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (document_id, savings) in linked_savings {
        if let Some(index) = document_indexes.get(&document_id) {
            documents[*index].linked_savings.push(savings);
        }
    }
    Ok(documents)
}

/// Atomically saves a payment document and replaces its linked savings accounts.
pub fn save_payment_document(
    connection: &Connection,
    id: Option<i64>,
    details: &PaymentDocumentDetails,
) -> Result<i64, DbError> {
    details.validate()?;
    let bank_exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM banks WHERE id = ?1)",
        [details.bank_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !bank_exists {
        return Err(DbError::PaymentDocumentBankUnavailable);
    }

    let transaction = connection.unchecked_transaction()?;
    if let Some(id) = id {
        let incompatible = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM incomes WHERE payment_document_id = ?1 AND (?2 <> 'debit' OR currency IS NOT ?3))",
            params![id, details.card_type.code(), details.currency.map(Currency::code)],
            |row| row.get::<_, bool>(0),
        )?;
        if incompatible {
            return Err(DbError::IncomeCardInUse);
        }
        let applied_expenses = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM expenses
                WHERE payment_document_id = ?1
                  AND payment_document_balance_applied = 1
            )",
            [id],
            |row| row.get::<_, bool>(0),
        )?;
        if applied_expenses && details.card_type == PaymentCardType::Credit {
            return Err(DbError::AppliedExpenseCardType);
        }
        let incompatible_expenses = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM expenses
                WHERE payment_document_id = ?1
                  AND (
                      currency IS NOT ?2
                      OR (affects_deposit_rate = 1 AND ?3 = 0)
                  )
            )",
            params![
                id,
                details.currency.map(Currency::code),
                details.affects_deposit_rate
            ],
            |row| row.get::<_, bool>(0),
        )?;
        if incompatible_expenses {
            return Err(DbError::ExpenseCardInUse);
        }
    }
    for savings_id in &details.savings_ids {
        let savings_bank_id = transaction
            .query_row(
                "SELECT bank_id FROM savings WHERE id = ?1",
                [savings_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .ok_or(DbError::PaymentDocumentSavingsUnavailable)?;
        if savings_bank_id != details.bank_id {
            return Err(DbError::PaymentDocumentSavingsBankMismatch);
        }
    }

    let amount = canonical_amount(details.amount);
    let due_date = details
        .due_date
        .map(|date| date.format("%Y-%m-%d").to_string());
    let min_monthly_spend = details.min_monthly_spend.map(canonical_amount);
    let document_id = if let Some(id) = id {
        transaction.execute(
            "DELETE FROM document_rate_links WHERE document_id = ?1",
            [id],
        )?;
        let changed = transaction.execute(
            "UPDATE payment_documents
             SET bank_id = ?1, payment_system = ?2, card_type = ?3, amount = ?4,
                 due_date = ?5, grace_period_days = ?6, affects_deposit_rate = ?7,
                 min_monthly_spend = ?8, currency = ?9
             WHERE id = ?10",
            params![
                details.bank_id,
                details.payment_system.code(),
                details.card_type.code(),
                amount,
                due_date,
                details.grace_period_days,
                details.affects_deposit_rate,
                min_monthly_spend,
                details.currency.map(Currency::code),
                id
            ],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        id
    } else {
        transaction.execute(
            "INSERT INTO payment_documents (
                bank_id, payment_system, card_type, amount, due_date,
                grace_period_days, affects_deposit_rate, min_monthly_spend, currency
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                details.bank_id,
                details.payment_system.code(),
                details.card_type.code(),
                amount,
                due_date,
                details.grace_period_days,
                details.affects_deposit_rate,
                min_monthly_spend,
                details.currency.map(Currency::code)
            ],
        )?;
        transaction.last_insert_rowid()
    };

    for savings_id in &details.savings_ids {
        transaction.execute(
            "INSERT INTO document_rate_links (document_id, savings_id) VALUES (?1, ?2)",
            params![document_id, savings_id],
        )?;
    }
    transaction.commit()?;
    Ok(document_id)
}

/// Deletes a payment document and only its associated rate links.
pub fn delete_payment_document(connection: &Connection, id: i64) -> Result<(), DbError> {
    if connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM incomes WHERE payment_document_id = ?1)",
        [id],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(DbError::IncomeCardInUse);
    }
    if connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM expenses WHERE payment_document_id = ?1)",
        [id],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(DbError::ExpenseCardInUse);
    }
    let deleted = connection.execute("DELETE FROM payment_documents WHERE id = ?1", [id])?;
    if deleted == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows.into());
    }
    Ok(())
}

fn parse_decimal_text(value: String, column: usize) -> Result<Decimal, rusqlite::Error> {
    Decimal::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
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
            &format!("UPDATE {table} SET name = ?1, parent_id = ?2, is_active = ?3 WHERE id = ?4"),
            params![name, parent_id, is_active, id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        Ok(id)
    } else {
        connection.execute(
            &format!("INSERT INTO {table} (name, parent_id, is_active) VALUES (?1, ?2, ?3)"),
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
    let (transactions, table) = transaction_tables(kind);
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
    let mut statement = connection
        .prepare("SELECT id, name, comment FROM banks ORDER BY name COLLATE NOCASE, id")?;
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
    let current_version = detect_migration_version(connection)?;
    let latest_version = MIGRATIONS.last().map_or(0, |migration| migration.version);

    if current_version == Some(latest_version) {
        return Ok(());
    }

    let initial_data_version = database_data_version(connection)?;
    if current_version.is_some() {
        backup_before_migration(connection, database_path, latest_version)?;
    }

    let rebuilds_currency_tables = MIGRATIONS.iter().any(|migration| {
        migration.version == 8 && migration.version > current_version.unwrap_or(0)
    });
    if rebuilds_currency_tables {
        connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
    }

    let migration_result = (|| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if database_data_version(&transaction)? != initial_data_version
            || detect_migration_version(&transaction)? != current_version
        {
            return Err(DbError::ConcurrentDatabaseModification);
        }

        create_migration_history(&transaction)?;
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version > current_version.unwrap_or(0))
        {
            transaction.execute_batch(migration.sql)?;
            transaction.execute(
                "INSERT INTO schema_migrations (version) VALUES (?1)",
                [migration.version],
            )?;
        }

        if rebuilds_currency_tables {
            let foreign_key_violation = transaction
                .query_row(
                    "SELECT \"table\", rowid, parent FROM pragma_foreign_key_check LIMIT 1",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((table, row_id, parent)) = foreign_key_violation {
                return Err(DbError::ForeignKeyIntegrity(format!(
                    "таблица {table}, строка {row_id:?}, ссылка на {parent}"
                )));
            }
        }

        transaction.commit()?;
        Ok(())
    })();

    if rebuilds_currency_tables {
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    migration_result
}

fn detect_migration_version(connection: &Connection) -> Result<Option<i64>, DbError> {
    let history_exists = connection.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = 'schema_migrations'
        )",
        [],
        |row| row.get::<_, bool>(0),
    )?;

    if !history_exists {
        return if has_user_schema_objects(connection)? {
            Err(DbError::UnrecognizedSchema)
        } else {
            Ok(None)
        };
    }

    let mut statement =
        connection.prepare("SELECT version FROM schema_migrations ORDER BY version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if versions.is_empty() {
        return if has_user_schema_objects(connection)? {
            Err(DbError::UnrecognizedSchema)
        } else {
            Ok(None)
        };
    }

    let supported_version = MIGRATIONS.last().map_or(0, |migration| migration.version);
    let found_version = *versions.last().ok_or(DbError::InvalidMigrationHistory)?;
    if found_version > supported_version {
        return Err(DbError::UnsupportedSchemaVersion {
            found: found_version,
            supported: supported_version,
        });
    }

    if versions
        .iter()
        .enumerate()
        .any(|(index, version)| *version != index as i64 + 1)
    {
        return Err(DbError::InvalidMigrationHistory);
    }

    Ok(Some(found_version))
}

fn has_user_schema_objects(connection: &Connection) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM sqlite_master
            WHERE name NOT LIKE 'sqlite_%' AND name <> 'schema_migrations'
        )",
        [],
        |row| row.get(0),
    )
}

fn database_data_version(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.query_row("PRAGMA data_version", [], |row| row.get(0))
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
    use finance_core::{
        Currency, CurrencyDefinition, DomainError, InterestDestination, PaymentCardType,
        PaymentDocumentDetails, PaymentSystem, SavingsDetails, SavingsTerm, TransactionKind,
    };
    use rusqlite::{Connection, OpenFlags};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::{
        add_currency, delete_bank, delete_category, delete_currency, delete_payment_document,
        delete_savings, export_database_backup_from_path, get_app_setting,
        has_pending_database_import, initialize, list_currencies, list_payment_documents,
        list_payment_documents_filtered, list_savings, list_savings_filtered,
        list_transaction_amounts, list_transactions, load_analysis_transactions,
        load_analysis_transactions_by_category, restore_pending_database_import, save_bank,
        save_category, save_payment_document, save_savings, save_transaction, set_app_setting,
        set_currency_exchange_rate, set_primary_currency, stage_database_import, DbError,
        MIGRATIONS,
    };

    fn savings_details(bank_id: i64) -> SavingsDetails {
        SavingsDetails {
            bank_id,
            title: "Семейный резерв".to_owned(),
            amount: Decimal::from_str("12345678901234567890.1234").unwrap(),
            currency: None,
            base_rate: Decimal::from_str("5.25").unwrap(),
            rate_after_threshold: Some(Decimal::from_str("7.5").unwrap()),
            threshold_amount: Some(Decimal::from_str("50000.75").unwrap()),
            opened_date: NaiveDate::from_ymd_opt(2024, 11, 5),
            interest_accrual_date: NaiveDate::from_ymd_opt(2024, 12, 5),
            term_type: SavingsTerm::Limited,
            term_end_date: NaiveDate::from_ymd_opt(2027, 11, 18),
            interest_destination: InterestDestination::Reinvest,
            is_active: true,
        }
    }

    fn payment_document_details(bank_id: i64) -> PaymentDocumentDetails {
        PaymentDocumentDetails {
            bank_id,
            payment_system: PaymentSystem::MasterCard,
            card_type: PaymentCardType::Debit,
            amount: Decimal::from_str("12345678901234567890.1234").unwrap(),
            currency: None,
            due_date: None,
            grace_period_days: None,
            affects_deposit_rate: false,
            min_monthly_spend: None,
            savings_ids: Vec::new(),
        }
    }

    fn create_legacy_database(path: &std::path::Path, version: i64) -> Connection {
        let mut connection = Connection::open(path).unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );",
            )
            .unwrap();

        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= version)
        {
            transaction.execute_batch(migration.sql).unwrap();
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [migration.version],
                )
                .unwrap();

            match migration.version {
                1 => {
                    transaction
                        .execute(
                            "INSERT INTO app_settings (key, value) VALUES ('theme', 'dark')",
                            [],
                        )
                        .unwrap();
                }
                2 => {
                    transaction
                        .execute_batch(
                            "INSERT INTO banks (id, name, comment) VALUES (1, 'Банк Альфа', 'основной');
                             INSERT INTO income_categories (id, name, is_active)
                                VALUES (1, 'Зарплата', 0);
                             INSERT INTO expense_categories (id, name) VALUES (1, 'Дом');
                             INSERT INTO incomes (id, date, category_id, amount, currency, comment)
                                VALUES (1, '2024-01-02', 1, '1234.5600', 'USD', 'проверка дохода');
                             INSERT INTO expenses (id, date, category_id, amount, currency, comment)
                                VALUES (1, '2024-02-03', 1, '98.0700', 'RUB', 'проверка расхода');",
                        )
                        .unwrap();
                }
                3 => {
                    transaction
                        .execute_batch(
                            "INSERT INTO savings (
                                id, bank_id, title, amount, base_rate, term_type,
                                interest_destination, is_active
                             ) VALUES (
                                1, 1, 'Резерв', '999.0100', '5.00', 'unlimited', 'withdraw', 0
                             );",
                        )
                        .unwrap();
                }
                4..=10 => {}
                _ => unreachable!(),
            }
        }

        transaction.commit().unwrap();
        connection
    }

    fn migration_backup_count(directory: &std::path::Path) -> usize {
        std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("data.before-migration-v10-")
            })
            .count()
    }

    #[test]
    fn initialize_migrates_supported_versions_and_preserves_backup_and_data() {
        for version in 1..=8 {
            let directory = tempfile::tempdir().unwrap();
            let database_path = directory.path().join("data.db");
            drop(create_legacy_database(&database_path, version));

            let connection = initialize(&database_path).unwrap();
            let latest_version = connection
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(latest_version, 10);

            let income_count = connection
                .query_row("SELECT COUNT(*) FROM incomes", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(income_count, i64::from(version >= 2));
            if version >= 2 {
                let income = connection
                    .query_row(
                        "SELECT date, amount, currency, comment FROM incomes WHERE id = 1",
                        [],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .unwrap();
                assert_eq!(
                    income,
                    (
                        "2024-01-02".into(),
                        "1234.5600".into(),
                        "USD".into(),
                        "проверка дохода".into()
                    )
                );

                let bank = connection
                    .query_row("SELECT name, comment FROM banks WHERE id = 1", [], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .unwrap();
                assert_eq!(bank, ("Банк Альфа".into(), "основной".into()));

                let income_category = connection
                    .query_row(
                        "SELECT name, is_active FROM income_categories WHERE id = 1",
                        [],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
                    )
                    .unwrap();
                assert_eq!(income_category, ("Зарплата".into(), false));

                let expense_category = connection
                    .query_row(
                        "SELECT name FROM expense_categories WHERE id = 1",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap();
                assert_eq!(expense_category, "Дом");

                let expense = connection
                    .query_row(
                        "SELECT date, amount, currency, comment FROM expenses WHERE id = 1",
                        [],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .unwrap();
                assert_eq!(
                    expense,
                    (
                        "2024-02-03".into(),
                        "98.0700".into(),
                        "RUB".into(),
                        "проверка расхода".into()
                    )
                );
                let expense_card_state = connection
                    .query_row(
                        "SELECT payment_document_id, affects_deposit_rate,
                                payment_document_balance_applied
                         FROM expenses WHERE id = 1",
                        [],
                        |row| {
                            Ok((
                                row.get::<_, Option<i64>>(0)?,
                                row.get::<_, bool>(1)?,
                                row.get::<_, bool>(2)?,
                            ))
                        },
                    )
                    .unwrap();
                assert_eq!(expense_card_state, (None, false, false));
            }

            let savings_count = connection
                .query_row("SELECT COUNT(*) FROM savings", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(savings_count, i64::from(version >= 3));
            if version >= 3 {
                let savings = connection
                    .query_row(
                        "SELECT title, amount, base_rate, term_type, interest_destination, is_active
                         FROM savings WHERE id = 1",
                        [],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, bool>(5)?,
                            ))
                        },
                    )
                    .unwrap();
                assert_eq!(
                    savings,
                    (
                        "Резерв".into(),
                        "999.0100".into(),
                        "5.00".into(),
                        "unlimited".into(),
                        "withdraw".into(),
                        false
                    )
                );
            }
            drop(connection);

            let backup_path = std::fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("data.before-migration-v10-")
                })
                .expect("upgrade should create a backup")
                .path();
            let backup = Connection::open(backup_path).unwrap();
            let backup_version = backup
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(backup_version, version);
            let setting = backup
                .query_row(
                    "SELECT value FROM app_settings WHERE key = 'theme'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap();
            assert_eq!(setting, "dark");
            if version >= 2 {
                let amount = backup
                    .query_row("SELECT amount FROM incomes WHERE id = 1", [], |row| {
                        row.get::<_, String>(0)
                    })
                    .unwrap();
                assert_eq!(amount, "1234.5600");
            }
            if version >= 3 {
                let amount = backup
                    .query_row("SELECT amount FROM savings WHERE id = 1", [], |row| {
                        row.get::<_, String>(0)
                    })
                    .unwrap();
                assert_eq!(amount, "999.0100");
            }
            drop(backup);

            assert_eq!(migration_backup_count(directory.path()), 1);
            drop(initialize(&database_path).unwrap());
            assert_eq!(migration_backup_count(directory.path()), 1);
        }
    }

    #[test]
    fn migrating_legacy_expense_with_debit_card_does_not_change_balance() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let legacy = create_legacy_database(&database_path, 9);
        let bank_id = 1;
        let mut card_details = payment_document_details(bank_id);
        card_details.amount = Decimal::from(200);
        card_details.currency = Some(Currency::Rub);
        let card_id = save_payment_document(&legacy, None, &card_details).unwrap();
        legacy
            .execute(
                "UPDATE expenses SET payment_document_id = ?1 WHERE id = 1",
                [card_id],
            )
            .unwrap();
        drop(legacy);

        let database = initialize(&database_path).unwrap();
        let expenses = list_transactions(
            &database,
            TransactionKind::Expense,
            None,
            None,
            None,
            None,
            "",
        )
        .unwrap();
        assert_eq!(expenses.len(), 1);
        assert_eq!(expenses[0].payment_document_id, Some(card_id));
        assert!(!expenses[0].payment_document_balance_applied);
        assert_eq!(
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .iter()
                .find(|card| card.id == card_id)
                .unwrap()
                .amount,
            Decimal::from(200)
        );

        super::delete_transaction(&database, TransactionKind::Expense, expenses[0].id).unwrap();
        assert_eq!(
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .iter()
                .find(|card| card.id == card_id)
                .unwrap()
                .amount,
            Decimal::from(200)
        );
    }

    #[test]
    fn initialize_does_not_modify_unrecognized_unversioned_database() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE legacy_records (value TEXT NOT NULL);
                 INSERT INTO legacy_records (value) VALUES ('keep me');",
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            initialize(&database_path),
            Err(DbError::UnrecognizedSchema)
        ));

        let connection = Connection::open(&database_path).unwrap();
        let value = connection
            .query_row("SELECT value FROM legacy_records", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        let migration_table_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(value, "keep me");
        assert_eq!(migration_table_count, 0);
    }

    #[test]
    fn initialize_does_not_modify_database_with_incomplete_migration_history() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 INSERT INTO schema_migrations (version) VALUES (1), (3);
                 CREATE TABLE app_settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO app_settings (key, value) VALUES ('theme', 'dark');",
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            initialize(&database_path),
            Err(DbError::InvalidMigrationHistory)
        ));

        let connection = Connection::open(&database_path).unwrap();
        let setting = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'theme'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let migration_count = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let banks_table_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'banks'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(setting, "dark");
        assert_eq!(migration_count, 2);
        assert_eq!(banks_table_count, 0);
        assert_eq!(migration_backup_count(directory.path()), 0);
    }

    #[test]
    fn initialize_rolls_back_failed_migration_and_keeps_backup() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let connection = create_legacy_database(&database_path, 1);
        connection
            .execute_batch(
                "CREATE TABLE savings (marker TEXT NOT NULL);
                 INSERT INTO savings (marker) VALUES ('preserve');",
            )
            .unwrap();
        drop(connection);

        assert!(initialize(&database_path).is_err());

        let connection = Connection::open(&database_path).unwrap();
        let schema_version = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let marker = connection
            .query_row("SELECT marker FROM savings", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        let banks_table_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'banks'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(schema_version, 1);
        assert_eq!(marker, "preserve");
        assert_eq!(banks_table_count, 0);
        assert_eq!(migration_backup_count(directory.path()), 1);
    }

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
        let schema_version = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(schema_version, 10);
    }

    #[test]
    fn migration_v9_preserves_existing_savings_without_assuming_currency() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        drop(create_legacy_database(&database_path, 8));

        let connection = initialize(&database_path).unwrap();
        let savings = list_savings(&connection, None, None, "").unwrap();
        let backup_version = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("data.before-migration-v10-")
            })
            .map(|entry| Connection::open(entry.path()).unwrap())
            .map(|backup| {
                backup
                    .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap()
            });

        assert_eq!(savings.len(), 1);
        assert_eq!(savings[0].currency, None);
        assert_eq!(backup_version, Some(8));
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

        assert_eq!(migration_count, 10);
        assert_eq!(foreign_keys_enabled, 1);
    }

    #[test]
    fn currency_directory_seeds_rub_as_primary_and_usd_without_an_assumed_rate() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();

        assert_eq!(
            list_currencies(&connection).unwrap(),
            vec![
                CurrencyDefinition {
                    code: "RUB".to_owned(),
                    name: "Российский рубль".to_owned(),
                    is_primary: true,
                    rate_to_primary: None,
                    rate_date: None,
                },
                CurrencyDefinition {
                    code: "USD".to_owned(),
                    name: "Доллар США".to_owned(),
                    is_primary: false,
                    rate_to_primary: None,
                    rate_date: None,
                },
            ]
        );
    }

    #[test]
    fn delete_currency_removes_unused_non_primary_currency() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();

        delete_currency(&connection, "EUR").unwrap();

        assert!(!list_currencies(&connection)
            .unwrap()
            .iter()
            .any(|currency| currency.code == "EUR"));
    }

    #[test]
    fn delete_currency_rejects_primary_currency() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();

        assert!(matches!(
            delete_currency(&connection, "RUB"),
            Err(DbError::PrimaryCurrencyCannotDelete)
        ));
    }

    #[test]
    fn delete_currency_rejects_currency_used_by_income() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        connection
            .execute_batch(
                "INSERT INTO income_categories (name) VALUES ('Зарплата');
                 INSERT INTO incomes (date, category_id, amount, currency)
                    VALUES ('2026-01-01', 1, '12.50', 'EUR');",
            )
            .unwrap();

        assert!(matches!(
            delete_currency(&connection, "EUR"),
            Err(DbError::CurrencyInUse)
        ));
    }

    #[test]
    fn delete_currency_rejects_currency_used_by_expense() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        connection
            .execute_batch(
                "INSERT INTO expense_categories (name) VALUES ('Покупки');
                 INSERT INTO expenses (date, category_id, amount, currency)
                    VALUES ('2026-01-01', 1, '12.50', 'EUR');",
            )
            .unwrap();

        assert!(matches!(
            delete_currency(&connection, "EUR"),
            Err(DbError::CurrencyInUse)
        ));
    }

    #[test]
    fn delete_currency_rejects_currency_used_by_payment_document() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        connection
            .execute_batch(
                "INSERT INTO banks (name) VALUES ('Банк');
                 INSERT INTO payment_documents (bank_id, payment_system, card_type, amount, currency)
                    VALUES (1, 'VISA', 'debit', '0', 'EUR');",
            )
            .unwrap();

        assert!(matches!(
            delete_currency(&connection, "EUR"),
            Err(DbError::CurrencyInUse)
        ));
    }

    #[test]
    fn currency_schema_accepts_additional_codes_in_transactions_and_cards() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        connection
            .execute_batch(
                "INSERT INTO income_categories (name) VALUES ('Зарплата');
                 INSERT INTO expense_categories (name) VALUES ('Покупки');
                 INSERT INTO banks (name) VALUES ('Банк');
                 INSERT INTO incomes (date, category_id, amount, currency)
                    VALUES ('2026-01-01', 1, '12.50', 'EUR');
                 INSERT INTO expenses (date, category_id, amount, currency)
                    VALUES ('2026-01-02', 1, '3.25', 'EUR');
                 INSERT INTO payment_documents (bank_id, payment_system, card_type, amount, currency)
                    VALUES (1, 'VISA', 'debit', '0', 'EUR');",
            )
            .unwrap();

        let currency_rows = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM incomes WHERE currency = 'EUR'),
                    (SELECT COUNT(*) FROM expenses WHERE currency = 'EUR'),
                    (SELECT COUNT(*) FROM payment_documents WHERE currency = 'EUR')",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(currency_rows, (1, 1, 1));
    }

    #[test]
    fn currency_directory_rejects_multiple_primary_currencies() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();

        assert!(connection
            .execute(
                "UPDATE currencies SET is_primary = 1 WHERE code = 'USD'",
                []
            )
            .is_err());
    }

    #[test]
    fn set_currency_exchange_rate_stores_decimal_value_and_effective_date() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        let rate_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();

        set_currency_exchange_rate(
            &connection,
            "EUR",
            Decimal::from_str("1.0842").unwrap(),
            rate_date,
        )
        .unwrap();

        let euro = list_currencies(&connection)
            .unwrap()
            .into_iter()
            .find(|currency| currency.code == "EUR")
            .unwrap();
        assert_eq!(
            euro.rate_to_primary,
            Some(Decimal::from_str("1.0842").unwrap())
        );
        assert_eq!(euro.rate_date, Some(rate_date));
    }

    #[test]
    fn set_currency_exchange_rate_rejects_primary_and_over_precise_rates() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        let rate_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();

        assert!(matches!(
            set_currency_exchange_rate(&connection, "RUB", Decimal::ONE, rate_date),
            Err(DbError::PrimaryCurrencyRateImmutable)
        ));
        assert!(matches!(
            set_currency_exchange_rate(
                &connection,
                "EUR",
                Decimal::from_str("1.0000000000001").unwrap(),
                rate_date,
            ),
            Err(DbError::Domain(DomainError::ExchangeRatePrecision))
        ));
    }

    #[test]
    fn set_primary_currency_overflow_rolls_back_without_changing_rates() {
        let directory = tempfile::tempdir().unwrap();
        let mut connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        set_currency_exchange_rate(
            &connection,
            "EUR",
            Decimal::from_str("0.000000000001").unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .unwrap();
        set_currency_exchange_rate(
            &connection,
            "USD",
            Decimal::MAX,
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        )
        .unwrap();

        let result = set_primary_currency(&mut connection, "EUR");
        assert!(matches!(
            result,
            Err(DbError::Domain(DomainError::ExchangeRateOverflow))
        ));
        let primary = list_currencies(&connection)
            .unwrap()
            .into_iter()
            .find(|currency| currency.is_primary)
            .unwrap();
        assert_eq!(primary.code, "RUB");
    }

    #[test]
    fn set_primary_currency_rebases_known_rates_and_preserves_unknown_rates() {
        let directory = tempfile::tempdir().unwrap();
        let mut connection = initialize(&directory.path().join("data.db")).unwrap();
        add_currency(&connection, "EUR", "Евро").unwrap();
        add_currency(&connection, "GBP", "Фунт стерлингов").unwrap();
        set_currency_exchange_rate(
            &connection,
            "USD",
            Decimal::from(100),
            NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
        )
        .unwrap();
        set_currency_exchange_rate(
            &connection,
            "EUR",
            Decimal::from(120),
            NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
        )
        .unwrap();

        set_primary_currency(&mut connection, "EUR").unwrap();
        let currencies = list_currencies(&connection).unwrap();
        let currency_by_code =
            |code: &str| currencies.iter().find(|currency| currency.code == code);

        assert_eq!(
            currency_by_code("RUB").unwrap().rate_to_primary,
            Some(Decimal::from_str("0.008333333333333333333333").unwrap())
        );
        assert_eq!(
            currency_by_code("USD").unwrap().rate_to_primary,
            Some(Decimal::from_str("0.833333333333333333333333").unwrap())
        );
        assert_eq!(currency_by_code("GBP").unwrap().rate_to_primary, None);
        assert!(currency_by_code("EUR").unwrap().is_primary);
        assert_eq!(
            currency_by_code("USD").unwrap().rate_date,
            NaiveDate::from_ymd_opt(2026, 1, 3)
        );

        let change_to_unknown = set_primary_currency(&mut connection, "GBP");
        assert!(matches!(
            change_to_unknown,
            Err(DbError::PrimaryCurrencyRateUnavailable)
        ));
        assert!(list_currencies(&connection)
            .unwrap()
            .iter()
            .any(|currency| currency.code == "EUR" && currency.is_primary));
    }

    #[test]
    fn currency_migration_preserves_transactions_cards_and_rate_links() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data.db");
        let legacy = create_legacy_database(&path, 7);
        legacy
            .execute_batch(
                "INSERT INTO payment_documents (
                    id, bank_id, payment_system, card_type, amount, currency,
                    affects_deposit_rate, min_monthly_spend
                 ) VALUES
                    (9, 1, 'VISA', 'debit', '200', 'USD', 0, NULL),
                    (10, 1, 'MIR', 'debit', '300', 'RUB', 1, '20');
                 INSERT INTO document_rate_links (id, document_id, savings_id)
                    VALUES (17, 10, 1);
                 UPDATE incomes SET payment_document_id = 9 WHERE id = 1;
                 UPDATE expenses
                    SET payment_document_id = 10, affects_deposit_rate = 1 WHERE id = 1;",
            )
            .unwrap();
        drop(legacy);

        let database = initialize(&path).unwrap();
        let linked_rows = database
            .query_row(
                "SELECT
                    (SELECT payment_document_id FROM incomes WHERE id = 1),
                    (SELECT payment_document_id FROM expenses WHERE id = 1),
                    (SELECT id FROM document_rate_links WHERE document_id = 10),
                    (SELECT currency FROM payment_documents WHERE id = 9)",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            linked_rows,
            (Some(9), Some(10), Some(17), Some("USD".to_owned()))
        );
        assert!(database
            .execute(
                "UPDATE payment_documents SET currency = 'EUR' WHERE id = 9",
                []
            )
            .is_err());
        let foreign_key_violations = database
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(foreign_key_violations, 0);
    }

    #[test]
    fn migration_adds_optional_card_currency_and_zero_debit_balance() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();

        let currency_column_exists = connection
            .query_row(
                "SELECT EXISTS (
                    SELECT 1 FROM pragma_table_info('payment_documents') WHERE name = 'currency'
                )",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap();
        let income_card_column_exists = connection
            .query_row(
                "SELECT EXISTS (
                    SELECT 1 FROM pragma_table_info('incomes') WHERE name = 'payment_document_id'
                )",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap();
        assert!(currency_column_exists);
        assert!(income_card_column_exists);

        save_bank(&connection, None, "Банк", "").unwrap();
        connection
            .execute(
                "INSERT INTO payment_documents (
                    bank_id, payment_system, card_type, amount, currency
                ) VALUES (1, 'VISA', 'debit', '0', 'RUB')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn migration_v6_preserves_cards_and_rate_link_ids() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data.db");
        let legacy = create_legacy_database(&path, 5);
        legacy.execute_batch("INSERT INTO payment_documents (id, bank_id, payment_system, card_type, amount, affects_deposit_rate, min_monthly_spend) VALUES (9, 1, 'MIR', 'debit', '10.25', 1, '20'); INSERT INTO document_rate_links (id, document_id, savings_id) VALUES (17, 9, 1);").unwrap();
        drop(legacy);
        let database = initialize(&path).unwrap();
        let card = database
            .query_row(
                "SELECT amount, currency FROM payment_documents WHERE id = 9",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(card, ("10.25".to_owned(), None));
        let link = database
            .query_row(
                "SELECT id, document_id, savings_id FROM document_rate_links",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(link, (17, 9, 1));
        assert_eq!(
            database
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(migration_backup_count(directory.path()), 1);
    }

    #[test]
    fn income_card_credits_follow_edits_transfer_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let db = initialize(&dir.path().join("data.db")).unwrap();
        let bank = save_bank(&db, None, "Банк", "").unwrap();
        let category =
            save_category(&db, TransactionKind::Income, None, "Доход", None, true).unwrap();
        let mut details = payment_document_details(bank);
        details.amount = Decimal::ZERO;
        details.currency = Some(Currency::Rub);
        let first = save_payment_document(&db, None, &details).unwrap();
        details.amount = Decimal::from(5);
        let second = save_payment_document(&db, None, &details).unwrap();
        let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let save = |id, amount, card| {
            super::save_transaction_with_card(
                &db,
                TransactionKind::Income,
                id,
                date,
                category,
                amount,
                Currency::Rub,
                "",
                card,
            )
        };
        let amount = Decimal::from_str("10.25").unwrap();
        let id = save(None, amount, Some(first)).unwrap();
        let balance = |card| {
            db.query_row(
                "SELECT amount FROM payment_documents WHERE id = ?1",
                [card],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        assert_eq!(balance(first), "10.25");
        assert_eq!(
            list_transactions(&db, TransactionKind::Income, None, None, None, None, "").unwrap()[0]
                .payment_document_id,
            Some(first)
        );
        save(Some(id), Decimal::from(12), Some(first)).unwrap();
        assert_eq!(balance(first), "12");
        save(Some(id), Decimal::from(7), Some(second)).unwrap();
        assert_eq!(balance(first), "0");
        assert_eq!(balance(second), "12");
        super::delete_transaction(&db, TransactionKind::Income, id).unwrap();
        assert_eq!(balance(second), "5");
    }

    #[test]
    fn income_card_rejects_invalid_cards_and_rolls_back_errors() {
        let dir = tempfile::tempdir().unwrap();
        let db = initialize(&dir.path().join("data.db")).unwrap();
        let bank = save_bank(&db, None, "Банк", "").unwrap();
        let category =
            save_category(&db, TransactionKind::Income, None, "Доход", None, true).unwrap();
        let mut details = payment_document_details(bank);
        details.amount = Decimal::ZERO;
        details.currency = Some(Currency::Rub);
        let first = save_payment_document(&db, None, &details).unwrap();
        details.amount = Decimal::MAX;
        let full = save_payment_document(&db, None, &details).unwrap();
        details.card_type = PaymentCardType::Credit;
        details.due_date = NaiveDate::from_ymd_opt(2024, 2, 1);
        details.grace_period_days = Some(30);
        let credit = save_payment_document(&db, None, &details).unwrap();
        let mut no_currency_details = payment_document_details(bank);
        no_currency_details.amount = Decimal::ZERO;
        no_currency_details.currency = None;
        let no_currency = save_payment_document(&db, None, &no_currency_details).unwrap();
        let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let save = |id, amount, currency, card| {
            super::save_transaction_with_card(
                &db,
                TransactionKind::Income,
                id,
                date,
                category,
                amount,
                currency,
                "",
                Some(card),
            )
        };
        assert!(matches!(
            save(None, Decimal::ONE, Currency::Rub, credit),
            Err(DbError::IncomeCardUnavailable)
        ));
        assert!(matches!(
            save(None, Decimal::ONE, Currency::Usd, first),
            Err(DbError::IncomeCardUnavailable)
        ));
        assert!(matches!(
            save(None, Decimal::ONE, Currency::Rub, full),
            Err(DbError::CardBalanceOverflow)
        ));
        assert!(matches!(
            save(None, Decimal::from_str("0.1").unwrap(), Currency::Rub, full),
            Err(DbError::CardBalanceOverflow)
        ));
        let id = save(None, Decimal::from(10), Currency::Rub, first).unwrap();
        assert!(matches!(
            save(Some(id), Decimal::from(20), Currency::Usd, first),
            Err(DbError::IncomeCardUnavailable)
        ));
        assert!(matches!(
            save(None, Decimal::ONE, Currency::Rub, no_currency),
            Err(DbError::IncomeCardUnavailable)
        ));
        assert!(db
            .execute(
                "INSERT INTO incomes (date, category_id, amount, currency, comment, payment_document_id)
                 VALUES ('2024-01-01', ?1, '1', 'USD', '', ?2)",
                rusqlite::params![category, first]
            )
            .is_err());
        assert!(db
            .execute("UPDATE incomes SET currency = 'USD' WHERE id = ?1", [id])
            .is_err());
        let unchanged =
            list_transactions(&db, TransactionKind::Income, None, None, None, None, "").unwrap();
        assert_eq!(unchanged.len(), 1);
        assert_eq!(unchanged[0].currency, Currency::Rub);
        assert_eq!(unchanged[0].amount, Decimal::from(10));
        assert!(matches!(
            save(Some(id), Decimal::ONE, Currency::Rub, full),
            Err(DbError::CardBalanceOverflow)
        ));
        let balance: String = db
            .query_row(
                "SELECT amount FROM payment_documents WHERE id = ?1",
                [first],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(balance, "10");
        assert!(matches!(
            delete_payment_document(&db, first),
            Err(DbError::IncomeCardInUse)
        ));
        db.execute(
            "UPDATE payment_documents SET amount = '0' WHERE id = ?1",
            [first],
        )
        .unwrap();
        assert!(matches!(
            super::delete_transaction(&db, TransactionKind::Income, id),
            Err(DbError::InsufficientCardBalance)
        ));
        let income =
            list_transactions(&db, TransactionKind::Income, None, None, None, None, "").unwrap();
        assert_eq!(income.len(), 1);
        assert_eq!(income[0].payment_document_id, Some(first));
        assert!(db
            .execute(
                "UPDATE payment_documents SET currency = 'USD' WHERE id = ?1",
                [first]
            )
            .is_err());
    }

    #[test]
    fn expenses_can_reference_cards_and_count_rate_with_debit_balance_updates() {
        let directory = tempfile::tempdir().unwrap();
        let database = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&database, None, "Банк", "").unwrap();
        let savings_id = save_savings(&database, None, &savings_details(bank_id)).unwrap();
        let category_id = save_category(
            &database,
            TransactionKind::Expense,
            None,
            "Покупки",
            None,
            true,
        )
        .unwrap();

        let mut debit_details = payment_document_details(bank_id);
        debit_details.amount = Decimal::from(200);
        debit_details.currency = Some(Currency::Rub);
        debit_details.affects_deposit_rate = true;
        debit_details.min_monthly_spend = Some(Decimal::from(100));
        debit_details.savings_ids = vec![savings_id];
        let debit_card_id = save_payment_document(&database, None, &debit_details).unwrap();

        let mut credit_details = payment_document_details(bank_id);
        credit_details.card_type = PaymentCardType::Credit;
        credit_details.amount = Decimal::from(5000);
        credit_details.currency = Some(Currency::Rub);
        credit_details.due_date = NaiveDate::from_ymd_opt(2024, 2, 1);
        credit_details.grace_period_days = Some(30);
        credit_details.affects_deposit_rate = true;
        credit_details.min_monthly_spend = Some(Decimal::from(100));
        credit_details.savings_ids = vec![savings_id];
        let credit_card_id = save_payment_document(&database, None, &credit_details).unwrap();

        let mut unqualified_details = payment_document_details(bank_id);
        unqualified_details.amount = Decimal::from(1000);
        unqualified_details.currency = Some(Currency::Rub);
        let unqualified_card_id =
            save_payment_document(&database, None, &unqualified_details).unwrap();
        let mut usd_details = payment_document_details(bank_id);
        usd_details.amount = Decimal::ZERO;
        usd_details.currency = Some(Currency::Usd);
        let usd_card_id = save_payment_document(&database, None, &usd_details).unwrap();
        let mut no_currency_details = payment_document_details(bank_id);
        no_currency_details.amount = Decimal::ZERO;
        no_currency_details.currency = None;
        let no_currency_card_id =
            save_payment_document(&database, None, &no_currency_details).unwrap();

        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let save = |id, currency, card_id, affects_deposit_rate| {
            super::save_transaction_with_card_and_rate_effect(
                &database,
                TransactionKind::Expense,
                id,
                date,
                category_id,
                Decimal::from(42),
                currency,
                "магазин",
                card_id,
                affects_deposit_rate,
            )
        };

        assert!(matches!(
            save(None, Currency::Rub, None, true),
            Err(DbError::ExpenseCardUnavailable)
        ));
        assert!(matches!(
            save(None, Currency::Rub, Some(unqualified_card_id), true),
            Err(DbError::ExpenseCardUnavailable)
        ));
        assert!(matches!(
            save(None, Currency::Rub, Some(usd_card_id), false),
            Err(DbError::ExpenseCardUnavailable)
        ));
        assert!(matches!(
            save(None, Currency::Rub, Some(no_currency_card_id), false),
            Err(DbError::ExpenseCardUnavailable)
        ));

        let expense_id = save(None, Currency::Rub, Some(debit_card_id), true).unwrap();
        assert_eq!(
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .iter()
                .find(|card| card.id == debit_card_id)
                .unwrap()
                .amount,
            Decimal::from(158)
        );
        assert!(matches!(
            save(Some(expense_id), Currency::Rub, Some(usd_card_id), false),
            Err(DbError::ExpenseCardUnavailable)
        ));
        let listed = list_transactions(
            &database,
            TransactionKind::Expense,
            None,
            None,
            None,
            None,
            "",
        )
        .unwrap();
        assert_eq!(listed[0].payment_document_id, Some(debit_card_id));
        assert!(listed[0].affects_deposit_rate);

        save_transaction(
            &database,
            TransactionKind::Expense,
            Some(expense_id),
            date,
            category_id,
            Decimal::from(24),
            Currency::Rub,
            "изменённый расход",
        )
        .unwrap();
        let preserved = list_transactions(
            &database,
            TransactionKind::Expense,
            None,
            None,
            None,
            None,
            "",
        )
        .unwrap();
        assert_eq!(preserved[0].payment_document_id, Some(debit_card_id));
        assert!(preserved[0].affects_deposit_rate);
        assert!(preserved[0].payment_document_balance_applied);
        assert_eq!(
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .iter()
                .find(|card| card.id == debit_card_id)
                .unwrap()
                .amount,
            Decimal::from(176)
        );

        let mut debit_as_credit = debit_details.clone();
        debit_as_credit.card_type = PaymentCardType::Credit;
        debit_as_credit.due_date = NaiveDate::from_ymd_opt(2024, 2, 1);
        debit_as_credit.grace_period_days = Some(30);
        assert!(matches!(
            save_payment_document(&database, Some(debit_card_id), &debit_as_credit),
            Err(DbError::AppliedExpenseCardType)
        ));
        assert!(database
            .execute(
                "UPDATE payment_documents
                 SET card_type = 'credit', due_date = '2024-02-01', grace_period_days = 30
                 WHERE id = ?1",
                [debit_card_id]
            )
            .is_err());

        save(Some(expense_id), Currency::Rub, Some(credit_card_id), true).unwrap();
        let moved = list_transactions(
            &database,
            TransactionKind::Expense,
            None,
            None,
            None,
            None,
            "",
        )
        .unwrap();
        assert_eq!(moved[0].payment_document_id, Some(credit_card_id));
        assert!(moved[0].affects_deposit_rate);
        assert!(!moved[0].payment_document_balance_applied);
        assert_eq!(
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .iter()
                .find(|card| card.id == debit_card_id)
                .unwrap()
                .amount,
            Decimal::from(200)
        );
        assert_eq!(
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .iter()
                .find(|card| card.id == credit_card_id)
                .unwrap()
                .amount,
            Decimal::from(5000)
        );
        save(None, Currency::Rub, Some(credit_card_id), true).unwrap();
        let credit_expenses = list_transactions(
            &database,
            TransactionKind::Expense,
            None,
            None,
            None,
            None,
            "",
        )
        .unwrap();
        assert!(credit_expenses
            .iter()
            .filter(|expense| expense.payment_document_id == Some(credit_card_id))
            .all(|expense| !expense.payment_document_balance_applied));
        assert_eq!(
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .iter()
                .find(|card| card.id == credit_card_id)
                .unwrap()
                .amount,
            Decimal::from(5000)
        );

        assert!(matches!(
            delete_payment_document(&database, credit_card_id),
            Err(DbError::ExpenseCardInUse)
        ));
        let mut disabled_rate = credit_details;
        disabled_rate.affects_deposit_rate = false;
        disabled_rate.min_monthly_spend = None;
        disabled_rate.savings_ids.clear();
        assert!(matches!(
            save_payment_document(&database, Some(credit_card_id), &disabled_rate),
            Err(DbError::ExpenseCardInUse)
        ));
        assert!(database
            .execute(
                "UPDATE payment_documents SET currency = 'USD' WHERE id = ?1",
                [credit_card_id]
            )
            .is_err());
        assert!(database
            .execute(
                "UPDATE payment_documents SET affects_deposit_rate = 0 WHERE id = ?1",
                [credit_card_id]
            )
            .is_err());
        assert!(database
            .execute(
                "UPDATE expenses SET payment_document_balance_applied = 1 WHERE id = ?1",
                [expense_id]
            )
            .is_err());
        assert!(database
            .execute(
                "INSERT INTO expenses (date, category_id, amount, currency, comment, payment_document_id, affects_deposit_rate)
                 VALUES ('2024-01-15', ?1, '1', 'USD', '', ?2, 0)",
                rusqlite::params![category_id, credit_card_id]
            )
            .is_err());
        assert!(database
            .execute(
                "UPDATE expenses SET currency = 'USD' WHERE id = ?1",
                [expense_id]
            )
            .is_err());
        assert!(database
            .execute(
                "INSERT INTO expenses (date, category_id, amount, currency, comment, affects_deposit_rate)
                 VALUES ('2024-01-15', ?1, '1', 'RUB', '', 1)",
                [category_id]
            )
            .is_err());
        assert!(database
            .execute(
                "INSERT INTO expenses (date, category_id, amount, currency, comment, payment_document_id, affects_deposit_rate)
                 VALUES ('2024-01-15', ?1, '1', 'RUB', '', ?2, 1)",
                rusqlite::params![category_id, unqualified_card_id]
            )
            .is_err());
    }

    #[test]
    fn expense_card_balance_changes_are_atomic_across_edit_move_unlink_and_delete() {
        let directory = tempfile::tempdir().unwrap();
        let database = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&database, None, "Банк", "").unwrap();
        let category_id = save_category(
            &database,
            TransactionKind::Expense,
            None,
            "Покупки",
            None,
            true,
        )
        .unwrap();
        let mut first_details = payment_document_details(bank_id);
        first_details.amount = Decimal::from(100);
        first_details.currency = Some(Currency::Rub);
        let first_card = save_payment_document(&database, None, &first_details).unwrap();
        let mut second_details = first_details.clone();
        second_details.amount = Decimal::from(80);
        let second_card = save_payment_document(&database, None, &second_details).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let save = |id, amount, card_id| {
            super::save_transaction_with_card_and_rate_effect(
                &database,
                TransactionKind::Expense,
                id,
                date,
                category_id,
                amount,
                Currency::Rub,
                "магазин",
                card_id,
                false,
            )
        };
        let balance = |card_id| {
            list_payment_documents(&database, None, None, None)
                .unwrap()
                .into_iter()
                .find(|card| card.id == card_id)
                .unwrap()
                .amount
        };

        let expense_id = save(None, Decimal::from(20), Some(first_card)).unwrap();
        assert_eq!(balance(first_card), Decimal::from(80));
        save(Some(expense_id), Decimal::from(25), Some(first_card)).unwrap();
        assert_eq!(balance(first_card), Decimal::from(75));
        save(Some(expense_id), Decimal::from(30), Some(second_card)).unwrap();
        assert_eq!(balance(first_card), Decimal::from(100));
        assert_eq!(balance(second_card), Decimal::from(50));
        save(Some(expense_id), Decimal::from(30), None).unwrap();
        assert_eq!(balance(second_card), Decimal::from(80));
        super::delete_transaction(&database, TransactionKind::Expense, expense_id).unwrap();
        assert_eq!(balance(second_card), Decimal::from(80));

        let deleted_expense = save(None, Decimal::from(10), Some(second_card)).unwrap();
        assert_eq!(balance(second_card), Decimal::from(70));
        super::delete_transaction(&database, TransactionKind::Expense, deleted_expense).unwrap();
        assert_eq!(balance(second_card), Decimal::from(80));

        database
            .execute(
                "UPDATE payment_documents SET amount = '5' WHERE id = ?1",
                [first_card],
            )
            .unwrap();
        assert!(matches!(
            save(None, Decimal::from(6), Some(first_card)),
            Err(DbError::InsufficientCardBalance)
        ));
        let expense_after_failed_create = save(None, Decimal::from(4), Some(first_card)).unwrap();
        assert_eq!(balance(first_card), Decimal::from(1));
        assert!(matches!(
            save(
                Some(expense_after_failed_create),
                Decimal::from(6),
                Some(first_card)
            ),
            Err(DbError::InsufficientCardBalance)
        ));
        assert_eq!(balance(first_card), Decimal::from(1));
        let saved_after_failed_edit = list_transactions(
            &database,
            TransactionKind::Expense,
            None,
            None,
            None,
            None,
            "",
        )
        .unwrap();
        assert_eq!(saved_after_failed_edit.len(), 1);
        assert_eq!(saved_after_failed_edit[0].amount, Decimal::from(4));
        assert!(saved_after_failed_edit[0].payment_document_balance_applied);
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
            None,
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

        let transaction_count = connection
            .query_row(
                "SELECT COUNT(*) FROM expenses WHERE category_id = ?1",
                [category_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(transaction_count, 1);
    }

    #[test]
    fn delete_category_removes_unused_income_category() {
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

        delete_category(&connection, TransactionKind::Income, category_id).unwrap();

        let category_count = connection
            .query_row(
                "SELECT COUNT(*) FROM income_categories WHERE id = ?1",
                [category_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(category_count, 0);
    }

    #[test]
    fn delete_category_removes_unused_expense_category() {
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

        delete_category(&connection, TransactionKind::Expense, category_id).unwrap();

        let category_count = connection
            .query_row(
                "SELECT COUNT(*) FROM expense_categories WHERE id = ?1",
                [category_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(category_count, 0);
    }

    #[test]
    fn delete_category_is_rejected_when_child_categories_exist() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let parent_id = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Основное",
            None,
            true,
        )
        .unwrap();
        let child_id = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Зарплата",
            Some(parent_id),
            true,
        )
        .unwrap();

        let error = delete_category(&connection, TransactionKind::Income, parent_id).unwrap_err();

        assert!(matches!(error, DbError::EntityInUse));
        let category_count = connection
            .query_row(
                "SELECT COUNT(*) FROM income_categories WHERE id IN (?1, ?2)",
                [parent_id, child_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(category_count, 2);
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
    fn list_transaction_amounts_uses_inclusive_dates_without_category_or_search_filters() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let income_category = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Зарплата",
            None,
            true,
        )
        .unwrap();
        let other_income_category = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Подарок",
            None,
            true,
        )
        .unwrap();
        let expense_category = save_category(
            &connection,
            TransactionKind::Expense,
            None,
            "Дом",
            None,
            true,
        )
        .unwrap();
        save_transaction(
            &connection,
            TransactionKind::Income,
            None,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            income_category,
            Decimal::from(10),
            Currency::Rub,
            "первый день",
        )
        .unwrap();
        save_transaction(
            &connection,
            TransactionKind::Income,
            None,
            NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            other_income_category,
            Decimal::from(20),
            Currency::Usd,
            "последний день",
        )
        .unwrap();
        save_transaction(
            &connection,
            TransactionKind::Expense,
            None,
            NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            expense_category,
            Decimal::from(4),
            Currency::Rub,
            "не подходит поиску",
        )
        .unwrap();
        save_transaction(
            &connection,
            TransactionKind::Expense,
            None,
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            expense_category,
            Decimal::from(6),
            Currency::Usd,
            "после периода",
        )
        .unwrap();

        let in_period = list_transaction_amounts(
            &connection,
            NaiveDate::from_ymd_opt(2024, 1, 1),
            NaiveDate::from_ymd_opt(2024, 1, 31),
        )
        .unwrap();
        let all_time = list_transaction_amounts(&connection, None, None).unwrap();
        let from_only =
            list_transaction_amounts(&connection, NaiveDate::from_ymd_opt(2024, 1, 31), None)
                .unwrap();
        let to_only =
            list_transaction_amounts(&connection, None, NaiveDate::from_ymd_opt(2024, 1, 1))
                .unwrap();

        assert_eq!(in_period.len(), 3);
        assert_eq!(all_time.len(), 4);
        assert_eq!(from_only.len(), 3);
        assert_eq!(to_only.len(), 1);
    }

    #[test]
    fn list_transaction_amounts_includes_transactions_added_after_previous_read() {
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

        let before_save = list_transaction_amounts(&connection, None, None).unwrap();
        save_transaction(
            &connection,
            TransactionKind::Income,
            None,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            category_id,
            Decimal::from(15),
            Currency::Rub,
            "новая операция",
        )
        .unwrap();
        let after_save = list_transaction_amounts(&connection, None, None).unwrap();

        assert!(before_save.is_empty());
        assert_eq!(after_save.len(), 1);
    }

    #[test]
    fn list_transaction_amounts_reports_malformed_decimal_values() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute(
                "INSERT INTO income_categories (name) VALUES ('Зарплата')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO incomes (date, category_id, amount, currency, comment)
                 VALUES ('2024-01-01', 1, 'повреждено', 'RUB', '')",
                [],
            )
            .unwrap();

        let error = list_transaction_amounts(&connection, None, None).unwrap_err();

        assert!(matches!(error, DbError::Sqlite(_)));
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
                    .contains("before-migration-v10")
            })
            .count();

        assert_eq!(preserved_setting, "dark");
        assert_eq!(backup_count, 1);
    }

    #[test]
    fn save_and_list_savings_preserves_decimal_precision_and_supports_filters() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        let other_bank_id = save_bank(&connection, None, "Другой банк", "").unwrap();
        let savings_id = save_savings(&connection, None, &savings_details(bank_id)).unwrap();

        let details = list_savings(&connection, Some(bank_id), Some(true), "семейный").unwrap();
        let account = &details[0];
        assert_eq!(details.len(), 1);
        assert_eq!(account.id, savings_id);
        assert_eq!(
            account.amount,
            Decimal::from_str("12345678901234567890.1234").unwrap()
        );
        assert_eq!(account.base_rate, Decimal::from_str("5.25").unwrap());
        assert_eq!(account.currency, None);
        assert_eq!(
            account.rate_after_threshold,
            Some(Decimal::from_str("7.5").unwrap())
        );
        assert_eq!(
            account.threshold_amount,
            Some(Decimal::from_str("50000.75").unwrap())
        );
        assert_eq!(account.term_type, SavingsTerm::Limited);
        assert_eq!(account.opened_date, savings_details(bank_id).opened_date);
        assert_eq!(
            account.interest_accrual_date,
            savings_details(bank_id).interest_accrual_date
        );

        let other_bank = savings_details(other_bank_id);
        save_savings(&connection, None, &other_bank).unwrap();
        assert_eq!(
            list_savings(&connection, Some(other_bank_id), Some(true), "")
                .unwrap()
                .len(),
            1
        );
        assert!(list_savings(&connection, None, Some(false), "")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn savings_currency_is_saved_and_filterable_without_changing_unknown_records() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();

        let mut rub = savings_details(bank_id);
        rub.currency = Some(Currency::Rub);
        save_savings(&connection, None, &rub).unwrap();
        let mut usd = savings_details(bank_id);
        usd.title = "Долларовый резерв".to_owned();
        usd.currency = Some(Currency::Usd);
        save_savings(&connection, None, &usd).unwrap();
        let unknown_id = save_savings(&connection, None, &savings_details(bank_id)).unwrap();

        assert_eq!(
            list_savings_filtered(
                &connection,
                Some(bank_id),
                Some(true),
                Some(Currency::Rub),
                "семейный",
            )
            .unwrap()
            .iter()
            .map(|account| account.currency)
            .collect::<Vec<_>>(),
            [Some(Currency::Rub)]
        );
        assert_eq!(
            list_savings_filtered(
                &connection,
                Some(bank_id),
                Some(true),
                Some(Currency::Usd),
                "долларовый",
            )
            .unwrap()
            .iter()
            .map(|account| account.currency)
            .collect::<Vec<_>>(),
            [Some(Currency::Usd)]
        );
        let all_savings = list_savings(&connection, None, None, "").unwrap();
        assert_eq!(all_savings.len(), 3);
        assert_eq!(
            all_savings
                .iter()
                .find(|account| account.id == unknown_id)
                .unwrap()
                .currency,
            None
        );
    }

    #[test]
    fn save_savings_rejects_incomplete_threshold_and_unknown_bank() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        let mut incomplete = savings_details(bank_id);
        incomplete.threshold_amount = None;

        let threshold_error = save_savings(&connection, None, &incomplete).unwrap_err();
        assert!(matches!(
            threshold_error,
            DbError::Domain(DomainError::IncompleteSavingsThreshold)
        ));

        let missing_bank_error = save_savings(&connection, None, &savings_details(-1)).unwrap_err();
        assert!(matches!(
            missing_bank_error,
            DbError::SavingsBankUnavailable
        ));
    }

    #[test]
    fn savings_account_update_and_delete_preserve_bank_restrictions() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        let savings_id = save_savings(&connection, None, &savings_details(bank_id)).unwrap();
        let mut updated = savings_details(bank_id);
        updated.title = "Резерв на отпуск".to_owned();
        updated.rate_after_threshold = None;
        updated.threshold_amount = None;
        updated.term_type = SavingsTerm::Unlimited;
        updated.term_end_date = None;
        updated.opened_date = None;
        updated.interest_accrual_date = NaiveDate::from_ymd_opt(2026, 2, 1);
        updated.interest_destination = InterestDestination::Withdraw;
        updated.is_active = false;

        save_savings(&connection, Some(savings_id), &updated).unwrap();
        assert!(matches!(
            delete_bank(&connection, bank_id),
            Err(DbError::EntityInUse)
        ));
        let account = list_savings(&connection, Some(bank_id), Some(false), "отпуск")
            .unwrap()
            .remove(0);
        assert_eq!(account.title, "Резерв на отпуск");
        assert_eq!(account.term_type, SavingsTerm::Unlimited);
        assert_eq!(account.opened_date, None);
        assert_eq!(
            account.interest_accrual_date,
            NaiveDate::from_ymd_opt(2026, 2, 1)
        );
        assert_eq!(account.interest_destination, InterestDestination::Withdraw);
        delete_savings(&connection, savings_id).unwrap();
        assert!(list_savings(&connection, None, None, "")
            .unwrap()
            .is_empty());
        delete_bank(&connection, bank_id).unwrap();
    }

    #[test]
    fn list_savings_reports_corrupted_decimal_instead_of_defaulting() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        save_savings(&connection, None, &savings_details(bank_id)).unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute("UPDATE savings SET amount = 'не число'", [])
            .unwrap();

        assert!(matches!(
            list_savings(&connection, None, None, ""),
            Err(DbError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
                ..
            )))
        ));
    }

    #[test]
    fn list_savings_reports_corrupted_date_instead_of_defaulting() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        save_savings(&connection, None, &savings_details(bank_id)).unwrap();
        let constraint_error = connection
            .execute("UPDATE savings SET opened_date = '2024-02-31'", [])
            .unwrap_err();
        assert!(matches!(
            constraint_error,
            rusqlite::Error::SqliteFailure(ref error, _)
                if error.code == rusqlite::ErrorCode::ConstraintViolation
        ));
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute("UPDATE savings SET opened_date = '2024-02-31'", [])
            .unwrap();

        assert!(matches!(
            list_savings(&connection, None, None, ""),
            Err(DbError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
                ..
            )))
        ));
    }

    #[test]
    fn migration_from_v4_adds_optional_savings_dates_and_creates_backup() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        drop(create_legacy_database(&database_path, 4));

        let connection = initialize(&database_path).unwrap();
        let savings_dates = connection
            .query_row(
                "SELECT opened_date, interest_accrual_date FROM savings WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .unwrap();
        let migration_version = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        drop(connection);

        let backup_path = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("data.before-migration-v10-")
            })
            .expect("upgrade from V4 should create a backup")
            .path();
        let backup = Connection::open(backup_path).unwrap();
        let backup_version = backup
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();

        assert_eq!(savings_dates, (None, None));
        assert_eq!(migration_version, 10);
        assert_eq!(backup_version, 4);
    }

    #[test]
    fn list_savings_reports_unknown_persisted_enum_values() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        save_savings(&connection, None, &savings_details(bank_id)).unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute("UPDATE savings SET term_type = 'future'", [])
            .unwrap();

        assert!(matches!(
            list_savings(&connection, None, None, ""),
            Err(DbError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
                ..
            )))
        ));
    }

    #[test]
    fn payment_document_crud_preserves_decimal_and_filters() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        let savings_id = save_savings(&connection, None, &savings_details(bank_id)).unwrap();
        let mut details = payment_document_details(bank_id);
        details.affects_deposit_rate = true;
        details.min_monthly_spend = Some(Decimal::from_str("10000.25").unwrap());
        details.savings_ids = vec![savings_id];

        let id = save_payment_document(&connection, None, &details).unwrap();
        let listed = list_payment_documents(
            &connection,
            Some(bank_id),
            Some(PaymentCardType::Debit),
            Some(PaymentSystem::MasterCard),
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].amount, details.amount);
        assert_eq!(listed[0].min_monthly_spend, details.min_monthly_spend);
        assert_eq!(listed[0].linked_savings[0].id, savings_id);

        let mut updated = details;
        updated.payment_system = PaymentSystem::Mir;
        updated.card_type = PaymentCardType::Credit;
        updated.amount = Decimal::from_str("5000.01").unwrap();
        updated.due_date = NaiveDate::from_ymd_opt(2026, 12, 1);
        updated.grace_period_days = Some(55);
        save_payment_document(&connection, Some(id), &updated).unwrap();

        let filtered = list_payment_documents(
            &connection,
            Some(bank_id),
            Some(PaymentCardType::Credit),
            Some(PaymentSystem::Mir),
        )
        .unwrap();
        assert_eq!(filtered[0].amount, updated.amount);
        assert_eq!(filtered[0].due_date, updated.due_date);
        assert_eq!(filtered[0].grace_period_days, Some(55));
        assert!(
            list_payment_documents(&connection, None, Some(PaymentCardType::Debit), None)
                .unwrap()
                .is_empty()
        );

        delete_payment_document(&connection, id).unwrap();
        let link_count = connection
            .query_row(
                "SELECT COUNT(*) FROM document_rate_links WHERE document_id = ?1",
                [id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(link_count, 0);
        assert_eq!(
            list_savings(&connection, Some(bank_id), None, "")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn payment_document_currency_filter_returns_only_matching_known_currency() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();

        let mut rub = payment_document_details(bank_id);
        rub.currency = Some(Currency::Rub);
        save_payment_document(&connection, None, &rub).unwrap();
        let mut usd = payment_document_details(bank_id);
        usd.payment_system = PaymentSystem::Mir;
        usd.currency = Some(Currency::Usd);
        save_payment_document(&connection, None, &usd).unwrap();
        save_payment_document(&connection, None, &payment_document_details(bank_id)).unwrap();

        let filtered = list_payment_documents_filtered(
            &connection,
            None,
            Some(PaymentCardType::Debit),
            Some(PaymentSystem::Mir),
            Some(Currency::Usd),
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].currency, Some(Currency::Usd));
        assert_eq!(
            list_payment_documents(&connection, None, None, None)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn payment_document_update_rolls_back_when_link_insert_fails() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let first_bank = save_bank(&connection, None, "Первый банк", "").unwrap();
        let second_bank = save_bank(&connection, None, "Второй банк", "").unwrap();
        let first_savings = save_savings(&connection, None, &savings_details(first_bank)).unwrap();
        let second_savings =
            save_savings(&connection, None, &savings_details(second_bank)).unwrap();
        let mut details = payment_document_details(first_bank);
        details.affects_deposit_rate = true;
        details.min_monthly_spend = Some(Decimal::from(1000));
        details.savings_ids = vec![first_savings];
        let id = save_payment_document(&connection, None, &details).unwrap();

        connection
            .execute_batch(
                "CREATE TRIGGER force_test_link_failure
                 BEFORE INSERT ON document_rate_links
                 BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
            )
            .unwrap();
        let mut updated = details;
        updated.bank_id = second_bank;
        updated.savings_ids = vec![second_savings];
        assert!(save_payment_document(&connection, Some(id), &updated).is_err());
        connection
            .execute_batch("DROP TRIGGER force_test_link_failure;")
            .unwrap();

        let persisted = list_payment_documents(&connection, Some(first_bank), None, None).unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].bank_id, first_bank);
        assert_eq!(persisted[0].linked_savings[0].id, first_savings);
    }

    #[test]
    fn payment_document_triggers_reject_direct_cross_bank_changes() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let first_bank = save_bank(&connection, None, "Первый банк", "").unwrap();
        let second_bank = save_bank(&connection, None, "Второй банк", "").unwrap();
        let first_savings = save_savings(&connection, None, &savings_details(first_bank)).unwrap();
        let second_savings =
            save_savings(&connection, None, &savings_details(second_bank)).unwrap();
        let mut details = payment_document_details(first_bank);
        details.affects_deposit_rate = true;
        details.min_monthly_spend = Some(Decimal::from(1000));
        details.savings_ids = vec![first_savings];
        let document_id = save_payment_document(&connection, None, &details).unwrap();

        assert!(connection
            .execute(
                "INSERT INTO document_rate_links (document_id, savings_id) VALUES (?1, ?2)",
                rusqlite::params![document_id, second_savings],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE document_rate_links SET savings_id = ?1 WHERE document_id = ?2",
                rusqlite::params![second_savings, document_id],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE payment_documents SET bank_id = ?1 WHERE id = ?2",
                rusqlite::params![second_bank, document_id],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE savings SET bank_id = ?1 WHERE id = ?2",
                rusqlite::params![second_bank, first_savings],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE payment_documents
                 SET affects_deposit_rate = 0, min_monthly_spend = NULL WHERE id = ?1",
                [document_id],
            )
            .is_err());
    }

    #[test]
    fn list_payment_documents_reports_corrupted_decimal() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        save_payment_document(&connection, None, &payment_document_details(bank_id)).unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute("UPDATE payment_documents SET amount = 'не число'", [])
            .unwrap();
        assert!(matches!(
            list_payment_documents(&connection, None, None, None),
            Err(DbError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
                ..
            )))
        ));
    }

    #[test]
    fn list_payment_documents_reports_unknown_payment_system() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let bank_id = save_bank(&connection, None, "Банк", "").unwrap();
        save_payment_document(&connection, None, &payment_document_details(bank_id)).unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute("UPDATE payment_documents SET payment_system = 'future'", [])
            .unwrap();

        assert!(matches!(
            list_payment_documents(&connection, None, None, None),
            Err(DbError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
                ..
            )))
        ));
    }

    #[test]
    fn load_analysis_transactions_filters_inclusive_dates_and_kind() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let connection = initialize(&database_path).unwrap();
        let income_category = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Зарплата",
            None,
            true,
        )
        .unwrap();
        let expense_category = save_category(
            &connection,
            TransactionKind::Expense,
            None,
            "Покупки",
            None,
            true,
        )
        .unwrap();
        for (kind, category_id, day, amount, currency) in [
            (
                TransactionKind::Income,
                income_category,
                NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
                Decimal::from(99),
                Currency::Rub,
            ),
            (
                TransactionKind::Income,
                income_category,
                NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
                Decimal::from(10),
                Currency::Rub,
            ),
            (
                TransactionKind::Income,
                income_category,
                NaiveDate::from_ymd_opt(2027, 1, 31).unwrap(),
                Decimal::from(20),
                Currency::Usd,
            ),
            (
                TransactionKind::Expense,
                expense_category,
                NaiveDate::from_ymd_opt(2027, 1, 15).unwrap(),
                Decimal::from(30),
                Currency::Rub,
            ),
            (
                TransactionKind::Expense,
                expense_category,
                NaiveDate::from_ymd_opt(2027, 2, 1).unwrap(),
                Decimal::from(40),
                Currency::Rub,
            ),
        ] {
            save_transaction(
                &connection,
                kind,
                None,
                day,
                category_id,
                amount,
                currency,
                "",
            )
            .unwrap();
        }

        let all_kinds = load_analysis_transactions(
            &database_path,
            NaiveDate::from_ymd_opt(2027, 1, 1),
            NaiveDate::from_ymd_opt(2027, 1, 31),
            None,
        )
        .unwrap();
        let income_only = load_analysis_transactions(
            &database_path,
            NaiveDate::from_ymd_opt(2027, 1, 1),
            NaiveDate::from_ymd_opt(2027, 1, 31),
            Some(TransactionKind::Income),
        )
        .unwrap();

        assert_eq!(all_kinds.len(), 3);
        assert_eq!(income_only.len(), 2);
        assert!(all_kinds
            .iter()
            .any(|entry| entry.amount == Decimal::from(10)));
        assert!(all_kinds
            .iter()
            .any(|entry| entry.amount == Decimal::from(20)));
    }

    #[test]
    fn load_analysis_transactions_by_category_filters_income_and_expenses_independently() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let connection = initialize(&database_path).unwrap();
        let income_category_one = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Доход 1",
            None,
            true,
        )
        .unwrap();
        let income_category_two = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Доход 2",
            None,
            true,
        )
        .unwrap();
        let expense_category_one = save_category(
            &connection,
            TransactionKind::Expense,
            None,
            "Расход 1",
            None,
            true,
        )
        .unwrap();
        let expense_category_two = save_category(
            &connection,
            TransactionKind::Expense,
            None,
            "Расход 2",
            None,
            true,
        )
        .unwrap();

        for (kind, category_id, amount) in [
            (TransactionKind::Income, income_category_one, 10),
            (TransactionKind::Income, income_category_two, 20),
            (TransactionKind::Expense, expense_category_one, 30),
            (TransactionKind::Expense, expense_category_two, 40),
        ] {
            save_transaction(
                &connection,
                kind,
                None,
                NaiveDate::from_ymd_opt(2027, 1, 15).unwrap(),
                category_id,
                Decimal::from(amount),
                Currency::Rub,
                "",
            )
            .unwrap();
        }

        let income_filter_only = load_analysis_transactions_by_category(
            &database_path,
            None,
            None,
            Some(income_category_two),
            None,
        )
        .unwrap();
        assert_eq!(
            income_filter_only
                .iter()
                .map(|entry| (entry.kind, entry.category_id))
                .collect::<Vec<_>>(),
            vec![
                (TransactionKind::Income, income_category_two),
                (TransactionKind::Expense, expense_category_one),
                (TransactionKind::Expense, expense_category_two),
            ]
        );

        let both_filters = load_analysis_transactions_by_category(
            &database_path,
            None,
            None,
            Some(income_category_two),
            Some(expense_category_one),
        )
        .unwrap();
        assert_eq!(
            both_filters
                .iter()
                .map(|entry| (entry.kind, entry.category_id))
                .collect::<Vec<_>>(),
            vec![
                (TransactionKind::Income, income_category_two),
                (TransactionKind::Expense, expense_category_one),
            ]
        );
        assert_eq!(income_category_one, expense_category_one);
        assert_eq!(income_category_two, expense_category_two);
    }

    #[test]
    fn load_analysis_transactions_does_not_create_missing_database() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("missing.db");

        assert!(load_analysis_transactions(&database_path, None, None, None).is_err());
        assert!(!database_path.exists());
    }

    #[test]
    fn load_analysis_transactions_reports_corrupted_decimal() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let connection = initialize(&database_path).unwrap();
        let category_id = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Зарплата",
            None,
            true,
        )
        .unwrap();
        save_transaction(
            &connection,
            TransactionKind::Income,
            None,
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            category_id,
            Decimal::ONE,
            Currency::Rub,
            "",
        )
        .unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute("UPDATE incomes SET amount = 'не число'", [])
            .unwrap();

        assert!(matches!(
            load_analysis_transactions(&database_path, None, None, None),
            Err(DbError::Sqlite(rusqlite::Error::FromSqlConversionFailure(
                ..
            )))
        ));
    }

    #[test]
    fn app_settings_can_be_read_and_updated() {
        let directory = tempfile::tempdir().unwrap();
        let database = initialize(&directory.path().join("data.db")).unwrap();

        assert_eq!(get_app_setting(&database, "theme").unwrap(), None);
        set_app_setting(&database, "theme", "dark").unwrap();

        assert_eq!(
            get_app_setting(&database, "theme").unwrap(),
            Some("dark".to_owned())
        );
    }

    #[test]
    fn export_database_backup_includes_committed_wal_changes() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let backup_path = directory.path().join("export.db");
        let database = initialize(&database_path).unwrap();
        let journal_mode = database
            .query_row("PRAGMA journal_mode = WAL", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        set_app_setting(&database, "theme", "light").unwrap();

        export_database_backup_from_path(&database_path, &backup_path).unwrap();

        let exported =
            Connection::open_with_flags(&backup_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            get_app_setting(&exported, "theme").unwrap(),
            Some("light".to_owned())
        );
    }

    #[test]
    fn export_database_backup_does_not_replace_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let backup_path = directory.path().join("export.db");
        drop(initialize(&database_path).unwrap());
        std::fs::write(&backup_path, b"keep existing file").unwrap();

        let error = export_database_backup_from_path(&database_path, &backup_path).unwrap_err();

        assert!(matches!(error, DbError::BackupDestinationExists));
        assert_eq!(std::fs::read(backup_path).unwrap(), b"keep existing file");
    }

    #[test]
    fn staged_import_preserves_current_database_and_keeps_a_backup() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let source_path = directory.path().join("source.db");

        let current = initialize(&database_path).unwrap();
        set_app_setting(&current, "theme", "light").unwrap();
        drop(current);
        let source = initialize(&source_path).unwrap();
        set_app_setting(&source, "theme", "dark").unwrap();
        drop(source);

        stage_database_import(&source_path, &database_path).unwrap();
        assert!(has_pending_database_import(&database_path));
        let current = Connection::open(&database_path).unwrap();
        assert_eq!(
            get_app_setting(&current, "theme").unwrap(),
            Some("light".to_owned())
        );
        drop(current);

        assert!(restore_pending_database_import(&database_path).unwrap());
        let imported = initialize(&database_path).unwrap();
        assert_eq!(
            get_app_setting(&imported, "theme").unwrap(),
            Some("dark".to_owned())
        );
        let backup_path = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("before-import"))
            })
            .expect("the replaced database should have a backup");
        let backup =
            Connection::open_with_flags(&backup_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            get_app_setting(&backup, "theme").unwrap(),
            Some("light".to_owned())
        );
    }

    #[test]
    fn invalid_import_does_not_change_the_current_database() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let source_path = directory.path().join("invalid.db");
        let database = initialize(&database_path).unwrap();
        set_app_setting(&database, "theme", "system").unwrap();
        std::fs::write(&source_path, b"not a SQLite database").unwrap();

        assert!(stage_database_import(&source_path, &database_path).is_err());
        assert!(!has_pending_database_import(&database_path));
        assert_eq!(
            get_app_setting(&database, "theme").unwrap(),
            Some("system".to_owned())
        );
    }

    #[test]
    fn importing_a_supported_older_schema_upgrades_the_staged_copy() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let source_path = directory.path().join("older.db");
        drop(create_legacy_database(&source_path, 7));

        stage_database_import(&source_path, &database_path).unwrap();
        assert!(restore_pending_database_import(&database_path).unwrap());

        let imported = initialize(&database_path).unwrap();
        let imported_version = imported
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(imported_version, MIGRATIONS.last().unwrap().version);
    }
}
