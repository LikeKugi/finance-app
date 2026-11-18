# Technical Specification: Personal Finance Desktop Application

## 1. General Information

### 1.1. Purpose
The application is designed for personal/family budget management: tracking income and expenses, managing savings deposits, tracking electronic payment documents (cards), generating analytical reports, and exporting the database.

### 1.2. Application Type
Native desktop application (Windows). No containerization (Docker) or virtualization. All data is stored in a local database installed together with the application.

### 1.3. Target Platform
- OS: Windows 10/11 (x64).
- **Technology stack: Rust.**
  - **GUI:** one of the native frameworks is recommended:
    - `egui` / `eframe` — lightweight, cross-platform, well-suited for tabular interfaces.
    - `iced` — declarative, with MVU architecture support.
    - `slint` — declarative UI with native performance.
    - `Tauri` — only if a WebView layer is acceptable; not recommended under strict "native" requirements, but may be considered as a compromise (Rust backend + system WebView, no Docker).
  - **Database:** `rusqlite` (embedded SQLite) or `sqlx` (with SQLite/PostgreSQL support). SQLite is recommended for local storage.
  - **ORM/Migrations:** `diesel` (with `diesel_migrations`) or `sqlx` with migrations.
  - **Serialization:** `serde`, `serde_json`.
  - **Export:** `csv`, `rust_xlsxwriter` (Excel), `zip` (for database archiving).
  - **Logging:** `tracing`, `log`.
  - **Configuration:** `config`, `toml`.
  - **Installer:** `cargo-wix` / `cargo-bundle` / NSIS wrapper for building MSI/EXE.

---

## 2. Architecture and Data Storage Requirements

### 2.1. Database
- Type: local relational DBMS — **SQLite** (embedded, file-based).
- Location: in the application's user directory (e.g., `%APPDATA%\FinanceApp\data.db`).
- Initialization: on first launch, the application automatically creates the database and applies migrations (`diesel_migrations` / `sqlx migrate`).
- Backup and export: see section 3.8.

### 2.2. Database Structure (minimum table set)
1. **`banks`** — bank directory:
   - `id` (PK), `name`, `comment`.
2. **`income_categories`** — income classifier:
   - `id` (PK), `name`, `parent_id` (hierarchy), `is_active`.
3. **`expense_categories`** — expense classifier:
   - `id` (PK), `name`, `parent_id`, `is_active`.
4. **`incomes`** — income transactions:
   - `id` (PK), `date`, `category_id` (FK), `amount`, `currency`, `comment`.
5. **`expenses`** — expense transactions:
   - `id` (PK), `date`, `category_id` (FK), `amount`, `currency`, `comment`.
6. **`savings`** — savings deposits/accounts:
   - `id` (PK), `bank_id` (FK), `title`, `amount`, `base_rate`, `rate_after_threshold`, `threshold_amount`, `term_type` (limited/unlimited), `term_end_date` (nullable), `interest_destination` (reinvest/withdraw), `is_active`.
7. **`payment_documents`** — electronic payment documents (cards):
   - `id` (PK), `bank_id` (FK), `payment_system` (VISA/MasterCard/MIR), `card_type` (debit/credit), `amount`, `due_date` (nullable), `grace_period_days` (nullable), `affects_deposit_rate` (bool), `min_monthly_spend` (nullable).
8. **`document_rate_links`** — link between a card and savings whose rate it affects:
   - `id` (PK), `document_id` (FK), `savings_id` (FK).

> Currencies (RUB/USD) are supported at the transaction level and displayed in the UI.

---

## 3. Functional Requirements

### 3.1. General UI
- Main window with tabs: "Income", "Expense", "Savings", "Payment Documents", "Analysis", "Directories".
- Toolbar: add, edit, delete, refresh, export.
- Bottom panel with aggregated indicators: "On hand", "Balance" (in RUB and USD).
- Display currency switcher.
- Filtering of records by date, period, week, group.
- Confirmation on record deletion.
- Input validation (required fields, numeric formats, dates).

### 3.2. "Income" Module
- Table: Date, Group (category), Amount, Comment.
- CRUD operations (create, read, update, delete).
- Category selection from the income classifier.
- Specification of the transaction currency.
- Filtering, sorting, total amounts for the period.

### 3.3. "Expense" Module
- Similar to "Income", but for expenses.
- Categories from the expense classifier.
- Detailed comments.
- Filtering and totals.

### 3.4. "Savings" Module
- Savings form:
  - Savings amount.
  - Base rate.
  - Rate after spending a certain amount (threshold rate).
  - Threshold spend amount.
  - Term: limited (with end date) or unlimited.
  - Bank name (selection from `banks`).
  - Interest destination: reinvestment or withdrawal to card.
- Tabular list with filtering by bank and status.
- Calculation of accrued interest (base) and potential income.
- Ability to link savings to payment documents (see 3.5).

### 3.5. "Electronic Payment Documents" Module
- Supported payment systems: VISA, MasterCard, MIR.
- Card fields:
  - Type: debit / credit.
  - Amount (balance/limit).
  - Due date (for credit cards).
  - Grace period in days (for credit cards).
  - Bank (selection from `banks`).
  - Flag "Can affect interest on deposits or accounts at the bank".
  - List of accounts/deposits whose interest is affected by the card (multi-select from the "Savings" module within the same bank) — via the `document_rate_links` table.
  - Required minimum monthly card spend.
- Tabular list of cards with filtering by bank, type, payment system.
- Validation: a card may only be linked to savings within the same bank.

### 3.6. "Directories" Module
- Management of income classifiers (add, edit, delete, activate/deactivate).
- Management of expense classifiers.
- Management of the bank directory.
- Protection against deletion of categories/banks used in transactions (or cascade update with confirmation).

### 3.7. "Analysis" Module
- Analysis period setup (date from / to).
- Modes: "Expense" / "Income" / "All".
- Summary table by category:
  - Amount in RUB and USD.
  - Percentage of total volume.
  - Monthly amount.
  - Annual forecast.
- Color indication of categories.
- Total rows: "Total" and "Of which by special categories".
- Report export to CSV/XLSX.

### 3.8. Database Export (new feature)
Several export modes must be implemented:

1. **Full database dump:**
   - Copy the `data.db` file to a user-selected directory with a timestamp (e.g., `data_2026-11-18_15-30.db`).
   - Optionally pack into a ZIP archive (`zip` crate) with an optional manifest file (schema version, date, checksum).

2. **Export to SQL dump:**
   - Generate an `.sql` file with `CREATE TABLE` and `INSERT` statements for all tables (use `rusqlite::backup` or a custom implementation via `SELECT` + `INSERT`).

3. **Export data to tabular format (CSV/XLSX):**
   - Export by module: income, expenses, savings, payment documents, directories.
   - Ability to select period and filters (as in the UI).
   - Use `csv` and `rust_xlsxwriter` crates.
   - Generate a separate file per module or a multi-sheet XLSX.

4. **Export UI:**
   - Separate "Export" menu item or toolbar button.
   - Selection dialog: export type (DB / SQL / CSV / XLSX), data range (all / for period), destination directory.
   - Progress indication and success/error message.
   - Logging of export operations.

5. **Import (optional, recommended):**
   - Ability to restore the database from a full dump (`.db` or ZIP) with schema compatibility check.
   - CSV/XLSX import — as agreed.

### 3.9. Additional Features
- Multi-currency support with display in the selected currency.
- Search across transactions.
- Hotkeys for quick record creation.
- Change log (audit) — optional.
- Error and operation logging (files in `%APPDATA%\FinanceApp\logs`).

---

## 4. User Interface Requirements
- Classic desktop interface (tables, toolbar, tabs).
- Localization: Russian language.
- Minimum resolution: 1280×720.
- Validation and confirmations on deletion.
- Native rendering (no WebView if `egui`/`iced`/`slint` is chosen).

---

## 5. Non-Functional Requirements
- **Native:** no Docker, web containers, or cloud services; everything local.
- **Performance:** opening modules and filtering — ≤ 1 second with up to 100,000 records.
- **Reliability:** database transactions (`rusqlite`/`sqlx`), integrity on abnormal termination.
- **Security:** optional — launch password, database encryption (e.g., `sqlcipher` via `rusqlite` with the `bundled-sqlcipher` feature).
- **Installation:** installer (MSI/EXE) via `cargo-wix`/NSIS; automatic database initialization and migrations on first launch.
- **Cross-platform (optional):** if `egui`/`iced`/`slint` is chosen — ability to build for Linux/macOS without code changes.

---

## 6. Development Stages (recommended)
1. Database and architecture design (Rust workspace: `core`, `db`, `ui`, `export`).
2. Implementation of "Income" and "Expense" modules + directories.
3. Implementation of "Savings" module + bank directory.
4. Implementation of "Payment Documents" module + linking with savings.
5. Implementation of "Analysis" module.
6. Implementation of the export subsystem (DB, SQL, CSV, XLSX).
7. Testing, installer build, documentation.

---

## 7. Acceptance Criteria
- All modules correspond to the functionality described.
- Data is stored in the local SQLite database and restored after restart.
- CRUD for classifiers and banks works; deletion of used records is protected.
- Correct operation of links between cards and savings within the same bank.
- Analytical report is generated for the selected period with percentages and forecast.
- Database export (full dump, SQL, CSV/XLSX) works and saves data correctly.
- The application runs without Docker and external servers; built with Rust.

---

## 8. Recommended Rust Project Structure (workspace)
```
finance-app/
├── Cargo.toml              # workspace
├── crates/
│   ├── core/               # domain models, business logic
│   ├── db/                 # rusqlite/sqlx, migrations, repositories
│   ├── ui/                 # egui/iced/slint interface
│   ├── export/             # export of DB, SQL, CSV, XLSX
│   └── app/                # main, dependency assembly
├── migrations/             # SQL migrations (diesel/sqlx)
├── assets/                 # icons, localization
└── installer/              # NSIS/cargo-wix configuration
```

---

If needed, I can additionally prepare:
- An ER diagram of the database.
- A list of user scenarios (use cases).
- Examples of migrations and repository structure in Rust.
- Mockups for the new modules ("Savings", "Payment Documents") taking into account the chosen GUI framework.
