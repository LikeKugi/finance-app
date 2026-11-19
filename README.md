# FinanceApp

FinanceApp is a local desktop application for personal and family finance management. The current implementation provides a native Russian-language window, local SQLite persistence, and functional income, expense, analysis, directory, savings, and payment document modules.

## Workspace

- `finance-app` — application entry point, user data paths, and logging.
- `finance-core` — transaction, currency, directory types, and exact decimal input handling.
- `finance-ui` — native `eframe` tables and Russian-language workflows.
- `finance-db` — SQLite connection setup, embedded migrations, and repositories.

Income and expense records support create, edit, delete with confirmation, inclusive date filters, category filters, text search, and exact RUB/USD totals. Directories support bank and hierarchical income/expense category management, activation, and deletion safeguards for categories in use. Savings support create, edit, delete with confirmation, bank/status/text filters, term and interest-destination fields, and exact decimal storage. Payment documents support create, edit, delete with confirmation, bank/system/card-type filters, credit payment dates and grace periods, and links to savings at the same bank. Analysis supports inclusive date ranges and independent income-category and expense-category selectors; each selector includes all categories (including inactive ones) using the same hierarchical labels as the income and expense filters. Both transaction groups remain visible, with exact category totals and shares grouped separately by transaction type and currency. Forecasts, currency conversion, and exports remain deferred.

Savings amounts have no currency field in the current schema. The savings page therefore displays the entered amount without a currency symbol and does not calculate accrued interest or potential income: opening dates, balance history, bank-specific accrual rules, and verified card-spend conditions are not available. A threshold rate is presented as conditional, not as confirmed yield.

Payment document amounts are stored as exact decimal text without a currency. For debit cards, the amount is the current balance; for credit cards, it is the credit limit and is not included in user-owned funds. Rate-influence settings describe a conditional opportunity only: the app does not associate expenses with cards, verify monthly spend thresholds, or calculate resulting yield. Payment systems are stored using the exact values `VISA`, `MasterCard`, and `MIR`; card numbers and last digits are not stored.

Analysis reads transactions from a separate read-only SQLite connection in one read transaction. Date boundaries are inclusive; empty boundaries mean the full available period. Category shares use the total for the same transaction type and currency, and RUB and USD are never combined. The current model does not define conversion rates, forecast methodology, or a confirmed criterion for special categories, so those results are not presented.

At startup, FinanceApp automatically upgrades databases with a recognized, contiguous migration history (V1–V3) to the current schema. Before upgrading, it creates an adjacent consistent SQLite backup named `data.before-migration-v4-<timestamp>.db`, then applies all pending migrations atomically. No user command is needed. A non-empty database without a recognized migration history, a damaged history, or a database from a newer app version is left unchanged and reported as an error; unsupported legacy formats are not guessed or rewritten.

Transaction dates and the income/expense date filters can be entered manually in `YYYY-MM-DD` format or selected from the calendar button. Date filters remain optional and can be cleared by deleting their text.

## Build and run

Install the Rust stable toolchain if needed:

```sh
rustup toolchain install stable
```

Build the app for the current machine (debug or optimized release):

```sh
cargo +stable build -p finance-app
cargo +stable build -p finance-app --release
```

Build all workspace crates in release mode:

```sh
cargo +stable build --workspace --release
```

To build a native macOS release executable in `target/macos` (requires Apple Command Line Tools):

```sh
cargo +stable build -p finance-app --release --locked --target-dir target/macos
```

The executable is written to `target/macos/release/finance-app`.

To cross-compile a Windows x64 GNU release executable, install the Rust target and ensure the GNU cross-linker is available in `PATH`:

```sh
rustup target add x86_64-pc-windows-gnu --toolchain stable
cargo +stable build -p finance-app --release --target x86_64-pc-windows-gnu
```

The executable is written to `target/x86_64-pc-windows-gnu/release/finance-app.exe`.

For a Windows x64 MSVC release executable, install the Visual Studio C++ Build Tools, then run:

```sh
rustup target add x86_64-pc-windows-msvc --toolchain stable
cargo +stable build -p finance-app --release --target x86_64-pc-windows-msvc
```

For development, run the app in debug mode on the current machine with one command. This form explicitly selects the stable toolchain and works even when `cargo +stable` is not supported by the active Cargo executable:

```sh
rustup run stable cargo run -p finance-app
```

On Windows, the database is stored at `%APPDATA%\FinanceApp\data.db` and logs are written under `%APPDATA%\FinanceApp\logs`. Other operating systems use their standard user configuration directory.