# Initial implementation decisions

- Start with a Rust workspace containing `finance-app`, `finance-ui`, and `finance-db`. Add `finance-core` and `finance-export` when their domain and export APIs are introduced; empty crates are not useful at this stage.
- Use `eframe` for the native desktop window and `rusqlite` with bundled SQLite for the local database.
- Store the database and rolling logs under the operating-system user's application data/configuration directory. On Windows this resolves under `%APPDATA%\FinanceApp`.
- The first embedded migration creates only the app-settings table and migration history. Financial tables and calculations are deferred to their corresponding implementation stages so their domain rules can be tested before they are introduced.
- This starting slice provides navigation placeholders only; it does not claim that financial modules are implemented.