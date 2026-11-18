# FinanceApp

FinanceApp is a local desktop application for personal and family finance management. The initial Rust workspace provides the native window shell, local SQLite initialization, and a migration baseline.

## Workspace

- `finance-app` — application entry point, user data paths, and logging.
- `finance-ui` — native `eframe` window and Russian-language module navigation.
- `finance-db` — SQLite connection setup and embedded migrations.

The Income, Expenses, Savings, Payment Documents, Analysis, and Directories tabs are scaffolding placeholders; financial CRUD, calculations, and exports have not yet been implemented.

## Build and run

With a Rust stable toolchain installed:

```sh
cargo run -p finance-app
```

On Windows, the database is stored at `%APPDATA%\FinanceApp\data.db` and logs are written under `%APPDATA%\FinanceApp\logs`. Other operating systems use their standard user configuration directory.