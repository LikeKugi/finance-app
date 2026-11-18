# FinanceApp

FinanceApp is a local desktop application for personal and family finance management. The current implementation provides a native Russian-language window, local SQLite persistence, and the first functional slice for income, expense, and directory management.

## Workspace

- `finance-app` — application entry point, user data paths, and logging.
- `finance-core` — transaction, currency, directory types, and exact decimal input handling.
- `finance-ui` — native `eframe` tables and Russian-language workflows.
- `finance-db` — SQLite connection setup, embedded migrations, and repositories.

Income and expense records support create, edit, delete with confirmation, inclusive date filters, category filters, text search, and exact RUB/USD totals. Directories support bank and hierarchical income/expense category management, activation, and deletion safeguards for categories in use. Savings, Payment Documents, Analysis, and exports are not implemented yet; no interest or balance formulas are assumed.

## Build and run

With a Rust stable toolchain installed:

```sh
cargo run -p finance-app
```

On Windows, the database is stored at `%APPDATA%\FinanceApp\data.db` and logs are written under `%APPDATA%\FinanceApp\logs`. Other operating systems use their standard user configuration directory.