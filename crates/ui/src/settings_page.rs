use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui;
use finance_db::Database;
use rfd::FileDialog;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    fn from_setting(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    const fn setting_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub const fn to_egui(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

enum OperationOutcome {
    BackupExported(PathBuf),
    ImportPrepared,
}

#[derive(Default)]
pub struct SettingsPage {
    theme: ThemePreference,
    message: Option<(String, bool)>,
    pending_import: Option<PathBuf>,
    operation: Option<Receiver<Result<OperationOutcome, String>>>,
    operation_message: Option<String>,
}

impl SettingsPage {
    pub fn load(database: &Database) -> Self {
        let mut page = Self::default();
        match finance_db::get_app_setting(database, "theme") {
            Ok(Some(value)) => match ThemePreference::from_setting(&value) {
                Some(theme) => page.theme = theme,
                None => {
                    page.message = Some((
                        "Не удалось прочитать сохранённую тему; используется системная тема".into(),
                        true,
                    ));
                }
            },
            Ok(None) => {}
            Err(error) => page.message = Some((error.to_string(), true)),
        }
        page
    }

    pub const fn theme(&self) -> ThemePreference {
        self.theme
    }

    pub fn render(
        &mut self,
        context: &egui::Context,
        ui: &mut egui::Ui,
        database: &Database,
        database_path: Option<&Path>,
    ) {
        let mut selected_theme = self.theme;
        ui.group(|ui| {
            ui.heading("Тема приложения");
            ui.horizontal(|ui| {
                ui.radio_value(&mut selected_theme, ThemePreference::System, "Системная");
                ui.radio_value(&mut selected_theme, ThemePreference::Light, "Светлая");
                ui.radio_value(&mut selected_theme, ThemePreference::Dark, "Тёмная");
            });
        });
        if selected_theme != self.theme {
            match finance_db::set_app_setting(database, "theme", selected_theme.setting_value()) {
                Ok(()) => {
                    self.theme = selected_theme;
                    context.set_theme(selected_theme.to_egui());
                }
                Err(error) => self.message = Some((error.to_string(), true)),
            }
        }

        ui.add_space(16.0);
        ui.group(|ui| {
            ui.heading("Резервная копия данных");
            ui.label(
                "Экспорт и импорт выполняются для всей базы FinanceApp в формате SQLite (.db).",
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.operation.is_none() && database_path.is_some(),
                        egui::Button::new("Экспортировать данные…"),
                    )
                    .clicked()
                {
                    self.start_export(context, database_path);
                }
                if ui
                    .add_enabled(
                        self.operation.is_none() && database_path.is_some(),
                        egui::Button::new("Импортировать данные…"),
                    )
                    .clicked()
                {
                    self.choose_import_file();
                }
            });
            if let Some(message) = &self.operation_message {
                ui.label(message);
            }
        });

        if let Some((message, is_error)) = &self.message {
            ui.add_space(12.0);
            ui.colored_label(
                if *is_error {
                    egui::Color32::RED
                } else {
                    egui::Color32::DARK_GREEN
                },
                message,
            );
        }

        self.render_import_confirmation(context, database_path);
    }

    fn start_export(&mut self, context: &egui::Context, database_path: Option<&Path>) {
        let Some(database_path) = database_path else {
            return;
        };
        let Some(destination) = FileDialog::new()
            .set_title("Экспорт данных FinanceApp")
            .set_file_name("financeapp-backup.db")
            .add_filter("База данных SQLite", &["db"])
            .save_file()
        else {
            return;
        };

        let database_path = database_path.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        self.operation = Some(receiver);
        self.operation_message = Some("Создаётся снимок базы данных…".into());
        let context = context.clone();
        thread::spawn(move || {
            let result = finance_db::export_database_backup_from_path(&database_path, &destination)
                .map(|()| OperationOutcome::BackupExported(destination))
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
            context.request_repaint();
        });
    }

    fn choose_import_file(&mut self) {
        let Some(source) = FileDialog::new()
            .set_title("Выберите резервную копию FinanceApp")
            .add_filter("База данных SQLite", &["db", "sqlite"])
            .pick_file()
        else {
            return;
        };
        self.pending_import = Some(source);
    }

    fn render_import_confirmation(
        &mut self,
        context: &egui::Context,
        database_path: Option<&Path>,
    ) {
        let Some(source) = self.pending_import.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Подтверждение импорта")
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(format!(
                    "Импорт заменит все текущие данные резервной копией «{}». Перед заменой будет создана резервная копия текущей базы. Приложение закроется и запустится с импортированными данными.",
                    file_name(&source)
                ));
                ui.horizontal(|ui| {
                    if ui.button("Импортировать и перезапустить").clicked() {
                        confirm = true;
                    }
                    if ui.button("Отмена").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            self.pending_import = None;
        }
        if !confirm {
            return;
        }
        self.pending_import = None;

        let Some(database_path) = database_path else {
            self.message = Some(("Импорт недоступен для этой базы данных".into(), true));
            return;
        };
        self.start_import(context, source, database_path);
    }

    fn start_import(&mut self, context: &egui::Context, source: PathBuf, database_path: &Path) {
        let database_path = database_path.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        self.operation = Some(receiver);
        self.operation_message = Some("Проверяется и подготавливается база для импорта…".into());
        let context = context.clone();
        thread::spawn(move || {
            let result = finance_db::stage_database_import(&source, &database_path)
                .map(|()| OperationOutcome::ImportPrepared)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
            context.request_repaint();
        });
    }

    pub fn poll_operation(&mut self) -> bool {
        let Some(receiver_result) = self.operation.as_ref().map(Receiver::try_recv) else {
            return false;
        };
        let result = match receiver_result {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.operation = None;
                self.operation_message = None;
                self.message = Some(("Фоновая операция завершилась без результата".into(), true));
                return false;
            }
        };

        self.operation = None;
        self.operation_message = None;
        match result {
            Ok(OperationOutcome::BackupExported(path)) => {
                self.message = Some((
                    format!("Резервная копия сохранена: {}", file_name(&path)),
                    false,
                ));
                false
            }
            Ok(OperationOutcome::ImportPrepared) => {
                self.message = Some((
                    "Импорт подготовлен; приложение перезапускается".into(),
                    false,
                ));
                true
            }
            Err(error) => {
                self.message = Some((error, true));
                false
            }
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("файл"))
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::ThemePreference;

    #[test]
    fn theme_setting_values_map_to_supported_preferences() {
        assert_eq!(
            ThemePreference::from_setting("system"),
            Some(ThemePreference::System)
        );
        assert_eq!(
            ThemePreference::from_setting("light"),
            Some(ThemePreference::Light)
        );
        assert_eq!(
            ThemePreference::from_setting("dark"),
            Some(ThemePreference::Dark)
        );
        assert_eq!(ThemePreference::from_setting("unknown"), None);
    }
}
