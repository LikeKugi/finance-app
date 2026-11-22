# FinanceApp

FinanceApp is a local desktop application for personal and family finance management. The current implementation provides a native Russian-language window, local SQLite persistence, and functional income, expense, analysis, directory, savings, and payment document modules.

The `Главная` start page opens on launch. It has no data table of its own: one centered vertical form titled `Добавление` creates an income or an expense without switching tabs or opening a modal dialog. A large amount input accepts arithmetic expressions; a large green `Добавить` button with a `CircleCheck` icon saves the record; a segmented `Доход / Расход` control keeps exactly one type active. Below it are full-width Small currency, category, card (`Зачислить на карту` for income, `Списать с карты` for expenses), and, for expenses, place selects. Records are saved with the current date and an empty comment; the date is shown near the amount. It deliberately shows no totals or balances because the formulas for "On hand" and "Balance" are not defined. Transfers are created from the Transfers page.

## Workspace

- `finance-app` — application entry point, user data paths, and logging.
- `finance-core` — transaction, currency, directory types, and exact decimal input handling.
- `finance-ui` — native `gpui-kit` tables and Russian-language workflows.
- `finance-db` — SQLite connection setup, embedded migrations, and repositories.

Income and expense records support create, edit, delete with confirmation, inclusive date filters, category filters, text search, and exact RUB/USD totals. Directories support bank, expense-place, and hierarchical income/expense category management, activation, and deletion safeguards for records in use. Expense places include optional multiline comments with a truncated table preview and full-text dialog. Savings support create, edit, delete with confirmation, bank/status/text filters, term and interest-destination fields, optional opening and interest-accrual dates, and exact decimal storage. Payment documents support create, edit, delete with confirmation, bank/system/card-type filters, credit payment dates and grace periods, and links to savings at the same bank. Analysis supports inclusive date ranges and independent income-category and expense-category selectors; each selector includes all categories (including inactive ones) using the same hierarchical labels as the income and expense filters. Selecting a category includes all of its descendants recursively in the corresponding transaction group, table, and chart; the other transaction group remains unfiltered unless its own category is selected. Category totals and shares remain separate by transaction type and currency. Forecasts, currency conversion, and exports remain deferred.

Database schema V8 adds a currency directory with three-letter uppercase codes, one primary currency, and optional dated rates quoted as units of the primary currency per one unit of the listed currency. Existing databases start with RUB as primary and USD with no assumed rate. The database API can add currencies, set rates, and atomically rebase known rates when the primary currency changes; unknown rates remain unknown and transaction amounts are never rewritten. User-entered rates allow at most 12 decimal places; rebased rates are rounded to 24 places using midpoint-to-even. The transaction domain/API, forms, and analysis still expose only RUB/USD, and the primary-currency selection is not yet surfaced in the UI.

Income and expense amount fields accept `+`, `-`, `*`, and `/` expressions, with multiplication and division evaluated before addition and subtraction. Each division is rounded to two decimal places (midpoint away from zero); the form warns that rounding error can accumulate, and only positive final amounts can be saved.

Savings amounts have no currency field in the current schema. The savings page therefore displays the entered amount without a currency symbol and does not calculate accrued interest or potential income. Opening and interest-accrual dates are optional, manually maintained reference dates; they do not represent a calculated or bank-verified accrual. Balance history, bank-specific accrual rules, and verified card-spend conditions are not available. A threshold rate is presented as conditional, not as confirmed yield.

Payment document amounts are stored as exact decimal text without a currency. For debit cards, the amount is the current balance; for credit cards, it is the credit limit and is not included in user-owned funds. Rate-influence settings describe a conditional opportunity only: the app does not associate expenses with cards, verify monthly spend thresholds, or calculate resulting yield. Payment systems are stored using the exact values `VISA`, `MasterCard`, and `MIR`; card numbers and last digits are not stored. Schema V11 adds an optional card name, but the UI does not expose it yet.

Analysis reads transactions from a separate read-only SQLite connection in one read transaction. Date boundaries are inclusive; empty boundaries mean the full available period. Selecting an income or expense category includes its entire descendant subtree, including inactive categories that still have historical transactions; each descendant appears as its own category in the table and chart. Category shares use the total for the same transaction type and currency, and RUB and USD are never combined. The current report does not apply the currency directory's rates; forecast methodology and a confirmed criterion for special categories are also undefined, so those results are not presented.

At startup, FinanceApp automatically upgrades databases with a recognized, contiguous migration history (V1–V11) to the current schema. Before upgrading, it creates an adjacent consistent SQLite backup named `data.before-migration-v11-<timestamp>.db`, then applies all pending migrations atomically. V11 adds the `expense_places` directory (required name, optional comment) and an optional expense reference; existing cards and expenses remain unnamed/unassigned. Place management is exposed in the directories screen, while expense-place selection is not yet available. No user command is needed. A non-empty database without a recognized migration history, a damaged history, or a database from a newer app version is left unchanged and reported as an error; unsupported legacy formats are not guessed or rewritten.

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

### Builds

#### GitHub Actions

The app version is defined once in `crates/app/Cargo.toml`. Windows executable metadata and both macOS bundle version fields are generated or populated from that version; tagged releases are rejected unless the tag matches it.

The workflow in `.github/workflows/build.yml` builds release artifacts on every push and pull request, and can also be started manually from the repository's **Actions** tab. It produces separate downloadable artifacts for Windows x64, macOS Apple Silicon (`arm64`), and macOS Intel (`x86_64`), with each macOS ZIP containing a versioned `FinanceApp.app` bundle.

For user-facing downloads, push a trusted version tag in the form `vMAJOR.MINOR.PATCH`. The separate `.github/workflows/release.yml` builds and publishes a GitHub Release containing `finance-app.exe` for Windows x64, `finance-app-macos-arm64.zip`, and `finance-app-macos-x86_64.zip`. The ordinary build workflow remains independent and continues to provide CI artifacts.

On Windows, download `finance-app.exe` from the GitHub Release and launch it directly; the release executable uses the GUI subsystem and does not open a terminal window. On macOS, download the ZIP matching the Mac's architecture, extract it, then open `FinanceApp.app` from Finder. Each archive contains one standalone `.app` bundle with its executable and icon.

Release builds are not signed or notarized. Windows SmartScreen or macOS Gatekeeper may still show an unknown-publisher or security warning; this release process does not guarantee their removal. The GitHub Release is a direct download and does not include an installer.

#### Windows

Build a Windows x64 release on Windows with Rust stable and the Windows SDK installed. The release build needs the SDK's `fxc.exe` shader compiler, which must be available in `PATH` or specified with `GPUI_FXC_PATH`:

```sh
cargo +stable build -p finance-app --release --locked
```

The executable is written to `target/release/finance-app.exe`. A release build uses the GUI subsystem, so launching the built EXE does not open a terminal window; debug builds keep the console, and launching from `cargo run` does not close an already-open terminal. The GitHub Actions workflow locates `fxc.exe` in the Windows SDK automatically. The Windows release build is performed on a Windows runner; cross-compiling this release from macOS is not supported by the current shader build dependency.

#### macOS

For a native macOS release, install Apple Command Line Tools:

```sh
cargo +stable build -p finance-app --release --locked --target-dir target/macos
```

The executable is written to `target/macos/release/finance-app`. This is a raw executable; the downloadable `.app` bundles are assembled by the tagged GitHub Release workflow.

For development, run the app in debug mode on the current machine with one command. This form explicitly selects the stable toolchain and works even when `cargo +stable` is not supported by the active Cargo executable:

```sh
rustup run stable cargo run -p finance-app
```

On Windows, the database is stored at `%APPDATA%\FinanceApp\data.db` and logs are written under `%APPDATA%\FinanceApp\logs`. Other operating systems use their standard user configuration directory.

## Transfers

The `Переводы` tab maintains a journal of account-balance changes: top-up, transfer between own accounts (card ↔ card, card ↔ savings, savings ↔ savings), and cash withdrawal. Saving or deleting a transfer adjusts the exact decimal balances of both participating accounts atomically; an edit first reverses the previous operation. Insufficient funds abort the whole operation.

Top-ups and withdrawals do not create income or expense records. The balance summary is therefore allowed to differ from the transactional `Анализ` report; the page states this explicitly. Only operations with the same explicitly configured currency are possible (RUB ↔ RUB or USD ↔ USD), and RUB and USD totals are never combined. Credit cards cannot participate because their amount is a credit limit. Inactive savings remain in journal history but cannot be selected for a new transfer.

Known limitations: no cross-currency conversion, no cash wallet or initial balances, no credit-card debt repayment, and no export of transfers outside a full database backup.
