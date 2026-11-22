# Current Architecture and Implementation Status

This document describes the architecture and functionality present in the repository, rather than the target architecture in `AGENTS.md`. It is based on the current source, crate manifests, migrations, and `docs/decisions.md`; update it when those change materially.

## Workspace and crate responsibilities

The Cargo workspace currently contains four crates:

| Crate | Current responsibility |
|---|---|
| `finance-app` (`crates/app`) | Application entry point; resolves the user's configuration directory, initializes logging and SQLite, then starts the UI. |
| `finance-core` (`crates/core`) | Domain types, validation, exact decimal parsing and formatting, and transaction and analysis value types. Most implementation is currently in `src/lib.rs`. |
| `finance-db` (`crates/db`) | SQLite initialization, schema migrations, queries, persistence operations, and database-level validation. Most implementation is currently in `src/lib.rs`; migration SQL is under `migrations/`. |
| `finance-ui` (`crates/ui`) | Native `gpui-kit` application and Russian-language screens. The window shell and tabs are in `src/gpui/shell.rs`; each tab is a separate page in `src/gpui/`, and framework-independent helpers are in `src/gpui/support.rs`. |

There is no `finance-export` crate in the current workspace. CSV, XLSX, and SQL export functionality is not implemented.

The current dependency direction is:

```text
finance-app -> finance-ui, finance-db
finance-ui  -> finance-core, finance-db
finance-db  -> finance-core
finance-core -> (no application crates)
```

The UI currently calls `finance-db` operations directly; a separate application-service or ports layer is not present. This is a description of the current code, not a recommendation to expand that coupling.

## Application startup and data flow

`crates/app/src/main.rs` resolves the user's configuration directory through `directories::BaseDirs`, creates a `FinanceApp` subdirectory, initializes daily rolling logs, restores a confirmed staged database import after closing the UI connection, and opens `data.db` through `finance_db::initialize`. The log configuration retains up to seven files. On Windows the application data directory is `%APPDATA%\FinanceApp`; other platforms use their standard configuration directory.

The initialized SQLite connection and database path are passed to `finance_ui::run`. The UI renders the module screens and calls database functions to load, validate, and persist user changes. SQLite is bundled through `rusqlite`, so normal application use does not depend on a separately installed SQLite server or network access.

## Persistence, migrations, and exact values

The database crate embeds the migration SQL files from `crates/db/migrations/` and applies versions V1 through V14. Initialization enables SQLite foreign keys on the connection and sets a five-second busy timeout. An empty database or a database with a recognized, contiguous migration history can be initialized or upgraded; an unrecognized, malformed, or newer schema is rejected without replacing the database.

Before upgrading a recognized older schema, the application creates an adjacent consistent backup using SQLite facilities (`VACUUM INTO`, as recorded in `docs/decisions.md`). Pending migrations and migration-history updates are applied atomically. Do not edit an applied migration; add a new version instead.

V11 adds nullable names to payment cards, an `expense_places` table with required names and nullable comments, and a nullable `expenses.expense_place_id` foreign key that restricts deletion of referenced places. Existing cards may remain unnamed, while card names are supported by the domain model, database CRUD API, and payment-card form. Expense places have domain and database CRUD support, are managed in the directories screen, and can be selected on expense forms, displayed in the expense table, and used as a list filter; existing expenses without a place remain valid.

V14 adds `prices`, storing positive exact-decimal amounts as canonical text, RUB/USD currency codes, ISO dates, and a required `expense_places` reference with restricted deletion. The directories screen includes a price section with CRUD, inclusive currency/place/date filters, sortable columns, and persisted resizable widths (except actions).

Financial amounts, rates, and thresholds are represented with `rust_decimal::Decimal` and stored as canonical decimal text rather than floating-point SQLite values. Income and expense operations support RUB and USD and are not converted implicitly. The currency directory and exchange-rate data exist in the database, but transaction forms and the transaction domain currently support only RUB and USD. Currency rates are user-provided; unknown rates remain unknown, and changing the primary currency does not rewrite transaction amounts.

## Implemented modules

- **Income and expenses:** create, edit, and delete with confirmation; category, date, currency, amount, and comment fields; optional expense-place selection; date/category/bank/payment-card/currency/text filtering and expense-place filtering; and exact totals. Payment-card options include every card when no bank is selected and only cards from the selected bank otherwise; changing the bank clears the card filter. Dates and date filters use inclusive bounds. Income and expense amounts are kept separate by currency, and the bank/card/place filters do not change the date-range balance.
- **Directories:** bank and expense-place management, hierarchical income and expense categories, and activation/deactivation with protections for records in use. Expense-place comments support multiline text and full-text viewing from the truncated table preview. The currency directory supports currency and dated-rate management, primary-currency selection, and safeguards against deleting a primary or used currency. Prices have a separate exact-decimal table with create/edit/delete confirmation, filters by currency, place, and inclusive date bounds, sorting by all data columns, and adjustable persisted widths except for actions.
- **Savings:** create, edit, and delete; bank, currency, status, and text filters; optional explicitly selected RUB/USD currency; rates and optional threshold values; term and interest destination; and optional opening/accrual reference dates. Existing savings retain unknown currency until set. Amounts and rates are stored exactly; savings do not calculate accrued interest or potential income.
- **Payment documents:** create, edit, and delete with an optional card name; the name is the first table column and can remain blank for existing cards. Names are included in the card selectors on income and expense forms. Includes bank, currency, payment-system, and card-type filters; debit and credit fields; and savings links restricted to the same bank. Currency filters include currency-unspecified cards only when set to `Все`. Payment-system values are stored as `VISA`, `MasterCard`, and `MIR`. Debit-card amounts mean current balance; credit-card amounts mean the credit limit, not user-owned funds. Income can be linked to a debit card and adjusts its balance atomically. New expenses linked to debit cards subtract their amount and reverse that balance change when edited, moved, unlinked, or deleted; credit-card limits remain unchanged. V10 marks pre-existing expenses as not applied and does not retroactively deduct them.
- **Analysis:** inclusive date filtering and all/income-only/expense-only modes; category totals and shares by transaction type and currency; and separate RUB and USD results without implicit conversion. Each transaction-type/currency group uses an independently sortable `DataTable` (category, operation count, amount, and share), with its total displayed below the table. Above the charts, the report shows filtered income, expenses, and their balance for each currency present; this balance is the difference between transactions, not the user's cash balance. Reports read both transaction tables from a single read-only SQLite snapshot.

- **Transfers:** a journal of top_up, transfer, and withdrawal operations that is the only source of balance history on this page. One atomic database operation adjusts the exact Decimal balances of up to two debit cards or savings accounts; editing and deleting reverse the previous legs. Top-up and withdrawal do not create income/expense transactions, so the balance summary may intentionally differ from the transactional Analysis report. Only RUB/RUB and USD/USD operations are supported; credit cards are excluded. The journal is filtered by inclusive date range, type, source, destination, currency, and text, and its column widths are stored under table_columns.transfers.v1.

The native UI exposes nine Russian-language tabs: home, income, expenses, savings, transfers, payment cards, analysis, directories, and settings. `Главная` is the start page: it centers one vertical form titled `Добавление` that creates an income or an expense directly; an exclusive `Доход / Расход` segmented control switches the record type, and the small selects below it change with the selected type. It shows short descriptions of income, expenses, and directories and links to every page. It displays no totals or balances, performs no financial calculations, and loads directory data for the forms when the page is activated, not on every render. Creating a transfer is available on the Transfers page. Settings persist light, dark, or system theme preference in the existing `app_settings` table, display the `finance-app` package version in the page footer, and export the whole database as a standalone SQLite backup using SQLite's backup API; imports validate integrity, foreign keys, and supported migration history, upgrade a staged copy, require confirmation, create a backup of the current database, and reopen the app with the imported data. Automated tests are primarily colocated in Rust source files, including the core, database, and UI crates, rather than a separate top-level `tests/` tree.

## Deferred functionality and known limitations

- Interest accrual estimates, yield projections, and verification of card-spend thresholds are deferred. Savings lack balance history and bank-specific accrual rules; card links and user-marked qualifying expenses do not prove that a threshold was met.
- Analysis forecasts, a special-category criterion, and currency conversion in reports are not implemented. RUB and USD are not combined into one amount.
- CSV, XLSX, and SQL dump export are not implemented; the settings screen currently imports and exports only complete SQLite `.db` backups.
- The database can store additional currencies and their rates, but transactions and transaction screens remain limited to RUB/USD.
- The current source provides a primary-currency selector in the directories screen. `README.md` still says that this selector is not surfaced in the UI; that statement is stale relative to `crates/ui/src/directories_page.rs`.
- This document does not assert that Windows packaging, installer behavior, or performance targets have been verified; those require their own platform-specific checks.

## Related documentation

- `AGENTS.md` — engineering constraints and target architecture.
- `docs/decisions.md` — accepted implementation decisions and financial semantics.
- `README.md` — product overview and build instructions.
- `docs/page-actions-and-filters.md` — page-level action and filter requirements.
- [Form guidelines](form-guidelines.md) — Russian-language target standard for all forms; it does not imply that the current forms already comply.