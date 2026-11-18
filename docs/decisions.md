# Initial implementation decisions

- Start with a Rust workspace containing `finance-app`, `finance-ui`, and `finance-db`; add `finance-core` when domain types and exact monetary input are introduced. Add `finance-export` only when export functionality is implemented.
- Use `eframe` for the native desktop window and `rusqlite` with bundled SQLite for the local database.
- Store the database and rolling logs under the operating-system user's application data/configuration directory. On Windows this resolves under `%APPDATA%\FinanceApp`.
- Migration V1 is immutable and contains the existing app-settings table. Migration V2 introduces banks, income/expense categories, and income/expense transactions; upgrading an existing versioned database first creates a consistent adjacent SQLite backup via `VACUUM INTO`.
- Transaction amounts are positive `rust_decimal::Decimal` values stored as canonical decimal `TEXT`, never calculated through SQLite floating-point aggregation. RUB and USD totals remain separate when no exchange rate is configured.
- The first functional slice covers income, expenses, and directories. Savings interest, card conditions, cross-currency totals, analysis formulas, and export behavior remain deferred until their semantics are agreed and implemented.