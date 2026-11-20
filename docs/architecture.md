# Current Architecture and Implementation Status

This document describes the architecture and functionality present in the repository, rather than the target architecture in `AGENTS.md`. It is based on the current source, crate manifests, migrations, and `docs/decisions.md`; update it when those change materially.

## Workspace and crate responsibilities

The Cargo workspace currently contains four crates:

| Crate | Current responsibility |
|---|---|
| `finance-app` (`crates/app`) | Application entry point; resolves the user's configuration directory, initializes logging and SQLite, then starts the UI. |
| `finance-core` (`crates/core`) | Domain types, validation, exact decimal parsing and formatting, and transaction and analysis value types. Most implementation is currently in `src/lib.rs`. |
| `finance-db` (`crates/db`) | SQLite initialization, schema migrations, queries, persistence operations, and database-level validation. Most implementation is currently in `src/lib.rs`; migration SQL is under `migrations/`. |
| `finance-ui` (`crates/ui`) | Native `eframe`/`egui` application and Russian-language screens. Shared application state is in `src/lib.rs`; feature-specific UI is split among page modules. |

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

`crates/app/src/main.rs` resolves the user's configuration directory through `directories::BaseDirs`, creates a `FinanceApp` subdirectory, initializes daily rolling logs, and opens `data.db` through `finance_db::initialize`. The log configuration retains up to seven files. On Windows the application data directory is `%APPDATA%\FinanceApp`; other platforms use their standard configuration directory.

The initialized SQLite connection and database path are passed to `finance_ui::run`. The UI renders the module screens and calls database functions to load, validate, and persist user changes. SQLite is bundled through `rusqlite`, so normal application use does not depend on a separately installed SQLite server or network access.

## Persistence, migrations, and exact values

The database crate embeds the migration SQL files from `crates/db/migrations/` and applies versions V1 through V8. Initialization enables SQLite foreign keys on the connection and sets a five-second busy timeout. An empty database or a database with a recognized, contiguous migration history can be initialized or upgraded; an unrecognized, malformed, or newer schema is rejected without replacing the database.

Before upgrading a recognized older schema, the application creates an adjacent consistent backup using SQLite facilities (`VACUUM INTO`, as recorded in `docs/decisions.md`). Pending migrations and migration-history updates are applied atomically. Do not edit an applied migration; add a new version instead.

Financial amounts, rates, and thresholds are represented with `rust_decimal::Decimal` and stored as canonical decimal text rather than floating-point SQLite values. Income and expense operations support RUB and USD and are not converted implicitly. The currency directory and exchange-rate data exist in the database, but transaction forms and the transaction domain currently support only RUB and USD. Currency rates are user-provided; unknown rates remain unknown, and changing the primary currency does not rewrite transaction amounts.

## Implemented modules

- **Income and expenses:** create, edit, and delete with confirmation; category, date, currency, amount, and comment fields; date/category/text filtering; and exact totals. Dates and date filters use inclusive bounds. Income and expense amounts are kept separate by currency.
- **Directories:** bank management, hierarchical income and expense categories, and activation/deactivation with protections for records in use. The currency directory supports currency and dated-rate management, primary-currency selection, and safeguards against deleting a primary or used currency.
- **Savings:** create, edit, and delete; bank, status, and text filters; rates and optional threshold values; term and interest destination; and optional opening/accrual reference dates. Amounts and rates are stored exactly. Savings currently have no currency field and do not calculate accrued interest or potential income.
- **Payment documents:** create, edit, and delete; bank, payment-system, and card-type filters; debit and credit fields; and savings links restricted to the same bank. Payment-system values are stored as `VISA`, `MasterCard`, and `MIR`. Debit-card amounts mean current balance; credit-card amounts mean the credit limit, not user-owned funds. Income can be linked to a debit card and adjusts its balance atomically; expenses can reference a card but do not change card balances.
- **Analysis:** inclusive date filtering and all/income-only/expense-only modes; category totals and shares by transaction type and currency; and separate RUB and USD results without implicit conversion. Reports read both transaction tables from a single read-only SQLite snapshot.

The native UI exposes six Russian-language tabs: income, expenses, savings, payment cards, analysis, and directories. Automated tests are primarily colocated in Rust source files, including the core, database, and UI crates, rather than a separate top-level `tests/` tree.

## Deferred functionality and known limitations

- Interest accrual estimates, yield projections, and verification of card-spend thresholds are deferred. Savings lack balance history and bank-specific accrual rules; card links and user-marked qualifying expenses do not prove that a threshold was met.
- Analysis forecasts, a special-category criterion, and currency conversion in reports are not implemented. RUB and USD are not combined into one amount.
- CSV, XLSX, SQL dump, and user-facing database export are not implemented; there is no export crate.
- The database can store additional currencies and their rates, but transactions and transaction screens remain limited to RUB/USD.
- The current source provides a primary-currency selector in the directories screen. `README.md` still says that this selector is not surfaced in the UI; that statement is stale relative to `crates/ui/src/directories_page.rs`.
- This document does not assert that Windows packaging, installer behavior, or performance targets have been verified; those require their own platform-specific checks.

## Related documentation

- `AGENTS.md` — engineering constraints and target architecture.
- `docs/decisions.md` — accepted implementation decisions and financial semantics.
- `README.md` — product overview and build instructions.
- `docs/page-actions-and-filters.md` — page-level action and filter requirements.