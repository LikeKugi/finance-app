use std::fs;

use anyhow::{Context, Result};
use directories::BaseDirs;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let base_dirs = BaseDirs::new().context("Не удалось определить каталог данных пользователя")?;
    let app_directory = base_dirs.config_dir().join("FinanceApp");
    let log_directory = app_directory.join("logs");
    fs::create_dir_all(&log_directory).context("Не удалось создать каталог журналов")?;

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("finance-app")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&log_directory)
        .context("Не удалось настроить журналы приложения")?;
    let (writer, _log_guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(writer)
        .try_init()
        .map_err(|error| anyhow::anyhow!("Не удалось запустить журналирование: {error:?}"))?;

    tracing::info!("Запуск FinanceApp");
    let database_path = app_directory.join("data.db");
    loop {
        finance_db::restore_pending_database_import(&database_path)
            .context("Не удалось восстановить резервную копию данных")?;
        let database = finance_db::initialize(&database_path).with_context(|| {
            format!(
                "Не удалось открыть базу данных: {}",
                database_path.display()
            )
        })?;
        // `run` возвращает управление после закрытия окна и освобождает соединение.
        finance_ui::run(database, database_path.clone());

        if finance_db::has_pending_database_import(&database_path) {
            finance_db::restore_pending_database_import(&database_path)
                .context("Не удалось восстановить выбранную резервную копию")?;
            continue;
        }
        break;
    }
    tracing::info!("Завершение FinanceApp");

    Ok(())
}
