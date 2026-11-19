use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use finance_core::{
    canonical_amount, AnalysisTransaction, Bank, Category, Currency, FinancialTransaction,
    InterestDestination, LinkedSavingsAccount, PaymentCardType, PaymentDocument,
    PaymentDocumentDetails, PaymentSystem, SavingsAccount, SavingsDetails, SavingsTerm,
    TransactionAmount, TransactionKind,
};
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
    #[error("База данных изменилась во время подготовки обновления; повторите запуск")]
    ConcurrentDatabaseModification,
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
    #[error("Банк платёжного документа не найден")]
    PaymentDocumentBankUnavailable,
    #[error("Одно или несколько накоплений не найдены")]
    PaymentDocumentSavingsUnavailable,
    #[error("Банк платёжного документа и выбранных накоплений должен совпадать")]
    PaymentDocumentSavingsBankMismatch,
    #[error("Начало периода не может быть позже его окончания")]
    InvalidAnalysisDateRange,
    #[error("Начало периода не может быть позже его окончания")]
    InvalidTransactionDateRange,
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
         ORDER BY t.date DESC, t.id DESC"
    );
    let from = from.map(|date| date.format("%Y-%m-%d").to_string());
    let to = to.map(|date| date.format("%Y-%m-%d").to_string());
    let mut statement = connection.prepare(&sql)?;
    let transactions = statement
        .query_map(params![from, to, category_id], |row| {
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

/// Lists savings accounts with optional bank, activity, and case-insensitive text filters.
pub fn list_savings(
    connection: &Connection,
    bank_id: Option<i64>,
    is_active: Option<bool>,
    search: &str,
) -> Result<Vec<SavingsAccount>, DbError> {
    let mut statement = connection.prepare(
        "SELECT s.id, s.bank_id, b.name, s.title, s.amount, s.base_rate,
                s.rate_after_threshold, s.threshold_amount, s.term_type, s.term_end_date,
                s.interest_destination, s.is_active
         FROM savings s JOIN banks b ON b.id = s.bank_id
         WHERE (?1 IS NULL OR s.bank_id = ?1)
           AND (?2 IS NULL OR s.is_active = ?2)
         ORDER BY b.name COLLATE NOCASE, s.title COLLATE NOCASE, s.id",
    )?;
    let rows = statement.query_map(params![bank_id, is_active], |row| {
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

        Ok(SavingsAccount {
            id: row.get(0)?,
            bank_id: row.get(1)?,
            bank_name: row.get(2)?,
            title: row.get(3)?,
            amount,
            base_rate,
            rate_after_threshold,
            threshold_amount,
            term_type,
            term_end_date,
            interest_destination,
            is_active: row.get(11)?,
        })
    })?;
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
    if let Some(id) = id {
        let changed = connection.execute(
            "UPDATE savings
             SET bank_id = ?1, title = ?2, amount = ?3, base_rate = ?4,
                 rate_after_threshold = ?5, threshold_amount = ?6, term_type = ?7,
                 term_end_date = ?8, interest_destination = ?9, is_active = ?10
             WHERE id = ?11",
            params![
                savings.bank_id,
                savings.title.trim(),
                amount,
                base_rate,
                rate_after_threshold,
                threshold_amount,
                savings.term_type.code(),
                term_end_date,
                savings.interest_destination.code(),
                savings.is_active,
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
                term_type, term_end_date, interest_destination, is_active
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                savings.bank_id,
                savings.title.trim(),
                amount,
                base_rate,
                rate_after_threshold,
                threshold_amount,
                savings.term_type.code(),
                term_end_date,
                savings.interest_destination.code(),
                savings.is_active
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
    let mut statement = connection.prepare(
        "SELECT document.id, document.bank_id, bank.name, document.payment_system,
                document.card_type, document.amount, document.due_date,
                document.grace_period_days, document.affects_deposit_rate,
                document.min_monthly_spend
         FROM payment_documents AS document
         JOIN banks AS bank ON bank.id = document.bank_id
         WHERE (?1 IS NULL OR document.bank_id = ?1)
           AND (?2 IS NULL OR document.card_type = ?2)
           AND (?3 IS NULL OR document.payment_system = ?3)
         ORDER BY bank.name COLLATE NOCASE, document.payment_system,
                  document.card_type, document.id",
    )?;
    let mut documents = statement
        .query_map(
            params![
                bank_id,
                card_type.map(PaymentCardType::code),
                payment_system.map(PaymentSystem::code)
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
                 min_monthly_spend = ?8
             WHERE id = ?9",
            params![
                details.bank_id,
                details.payment_system.code(),
                details.card_type.code(),
                amount,
                due_date,
                details.grace_period_days,
                details.affects_deposit_rate,
                min_monthly_spend,
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
                grace_period_days, affects_deposit_rate, min_monthly_spend
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                details.bank_id,
                details.payment_system.code(),
                details.card_type.code(),
                amount,
                due_date,
                details.grace_period_days,
                details.affects_deposit_rate,
                min_monthly_spend
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

    transaction.commit()?;
    Ok(())
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
        Currency, DomainError, InterestDestination, PaymentCardType, PaymentDocumentDetails,
        PaymentSystem, SavingsDetails, SavingsTerm, TransactionKind,
    };
    use rusqlite::Connection;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::{
        delete_bank, delete_category, delete_payment_document, delete_savings, initialize,
        list_payment_documents, list_savings, list_transaction_amounts, list_transactions,
        load_analysis_transactions, load_analysis_transactions_by_category, save_bank,
        save_category, save_payment_document, save_savings, save_transaction, DbError, MIGRATIONS,
    };

    fn savings_details(bank_id: i64) -> SavingsDetails {
        SavingsDetails {
            bank_id,
            title: "Семейный резерв".to_owned(),
            amount: Decimal::from_str("12345678901234567890.1234").unwrap(),
            base_rate: Decimal::from_str("5.25").unwrap(),
            rate_after_threshold: Some(Decimal::from_str("7.5").unwrap()),
            threshold_amount: Some(Decimal::from_str("50000.75").unwrap()),
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
                    .starts_with("data.before-migration-v4-")
            })
            .count()
    }

    #[test]
    fn initialize_migrates_supported_versions_and_preserves_backup_and_data() {
        for version in 1..=3 {
            let directory = tempfile::tempdir().unwrap();
            let database_path = directory.path().join("data.db");
            drop(create_legacy_database(&database_path, version));

            let connection = initialize(&database_path).unwrap();
            let latest_version = connection
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(latest_version, 4);

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
            }

            let savings_count = connection
                .query_row("SELECT COUNT(*) FROM savings", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(savings_count, i64::from(version == 3));
            if version == 3 {
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
                        .starts_with("data.before-migration-v4-")
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
            if version == 3 {
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
        assert_eq!(schema_version, 4);
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

        assert_eq!(migration_count, 4);
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
                    .contains("before-migration-v4")
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
        assert_eq!(
            account.rate_after_threshold,
            Some(Decimal::from_str("7.5").unwrap())
        );
        assert_eq!(
            account.threshold_amount,
            Some(Decimal::from_str("50000.75").unwrap())
        );
        assert_eq!(account.term_type, SavingsTerm::Limited);

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
}
