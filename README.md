# FinanceApp

FinanceApp is a local desktop application for personal and family finance management. The current implementation provides a native Russian-language window, local SQLite persistence, and the first functional slice for income, expense, and directory management.

## Workspace

- `finance-app` — application entry point, user data paths, and logging.
- `finance-core` — transaction, currency, directory types, and exact decimal input handling.
- `finance-ui` — native `eframe` tables and Russian-language workflows.
- `finance-db` — SQLite connection setup, embedded migrations, and repositories.

Income and expense records support create, edit, delete with confirmation, inclusive date filters, category filters, text search, and exact RUB/USD totals. Directories support bank and hierarchical income/expense category management, activation, and deletion safeguards for categories in use. Savings, Payment Documents, Analysis, and exports are not implemented yet; no interest or balance formulas are assumed.

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