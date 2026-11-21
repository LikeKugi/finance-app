//! Страница «Настройки» на `gpui-kit`: тема и резервное копирование всей базы SQLite.
//! Экспорт и подготовка
//! импорта выполняются в фоне через существующие функции `finance-db`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use finance_db::Database;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Theme, ThemeMode, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rfd::FileDialog;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    const ALL: [(Self, &'static str); 3] = [
        (Self::System, "Системная"),
        (Self::Light, "Светлая"),
        (Self::Dark, "Тёмная"),
    ];

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

    fn apply(self, window: &mut Window, cx: &mut App) {
        match self {
            Self::System => Theme::sync_system_appearance(Some(window), cx),
            Self::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Self::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }
}

enum OperationOutcome {
    BackupExported(PathBuf),
    ImportPrepared,
}

pub(crate) struct SettingsPage {
    database: Rc<Database>,
    database_path: PathBuf,
    theme: ThemePreference,
    message: Option<(String, bool)>,
    operation_message: Option<String>,
    busy: bool,
}

impl SettingsPage {
    pub(crate) fn new(
        database: Rc<Database>,
        database_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut page = Self {
            database,
            database_path,
            theme: ThemePreference::System,
            message: None,
            operation_message: None,
            busy: false,
        };
        match finance_db::get_app_setting(&page.database, "theme") {
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
        page.theme.apply(window, cx);
        page
    }

    fn select_theme(
        &mut self,
        theme: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if theme == self.theme {
            return;
        }
        match finance_db::set_app_setting(&self.database, "theme", theme.setting_value()) {
            Ok(()) => {
                self.theme = theme;
                theme.apply(window, cx);
            }
            Err(error) => self.message = Some((error.to_string(), true)),
        }
        cx.notify();
    }

    fn start_export(&mut self, cx: &mut Context<Self>) {
        let Some(destination) = FileDialog::new()
            .set_title("Экспорт данных FinanceApp")
            .set_file_name("financeapp-backup.db")
            .add_filter("База данных SQLite", &["db"])
            .save_file()
        else {
            return;
        };
        let database_path = self.database_path.clone();
        self.begin(
            "Создаётся снимок базы данных…",
            cx,
            move || {
                finance_db::export_database_backup_from_path(&database_path, &destination)
                    .map(|()| OperationOutcome::BackupExported(destination))
                    .map_err(|error| error.to_string())
            },
        );
    }

    fn choose_import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = FileDialog::new()
            .set_title("Выберите резервную копию FinanceApp")
            .add_filter("База данных SQLite", &["db", "sqlite"])
            .pick_file()
        else {
            return;
        };
        let description = format!(
            "Импорт заменит все текущие данные резервной копией «{}». Перед заменой будет создана резервная копия текущей базы. Приложение закроется и запустится с импортированными данными.",
            file_name(&source)
        );
        let this = cx.entity();
        window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
            let this = this.clone();
            let source = source.clone();
            alert
                .title("Подтверждение импорта")
                .description(description.clone())
                .ok_text("Импортировать и перезапустить")
                .cancel_text("Отмена")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |page, cx| page.start_import(source.clone(), cx));
                    true
                })
        });
    }

    fn start_import(&mut self, source: PathBuf, cx: &mut Context<Self>) {
        let database_path = self.database_path.clone();
        self.begin(
            "Проверяется и подготавливается база для импорта…",
            cx,
            move || {
                finance_db::stage_database_import(&source, &database_path)
                    .map(|()| OperationOutcome::ImportPrepared)
                    .map_err(|error| error.to_string())
            },
        );
    }

    /// Запускает фоновую операцию; интерфейс остаётся отзывчивым, повторный запуск заблокирован.
    fn begin(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
        job: impl FnOnce() -> Result<OperationOutcome, String> + Send + 'static,
    ) {
        self.busy = true;
        self.operation_message = Some(text.to_owned());
        self.message = None;
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |page, cx| page.finish(result, cx));
        })
        .detach();
        cx.notify();
    }

    fn finish(&mut self, result: Result<OperationOutcome, String>, cx: &mut Context<Self>) {
        self.busy = false;
        self.operation_message = None;
        match result {
            Ok(OperationOutcome::BackupExported(path)) => {
                self.message = Some((
                    format!("Резервная копия сохранена: {}", file_name(&path)),
                    false,
                ));
            }
            Ok(OperationOutcome::ImportPrepared) => {
                self.message = Some((
                    "Импорт подготовлен; приложение перезапускается".into(),
                    false,
                ));
                // Окно закрывается, `finance_ui::run` возвращается, а `app` применяет импорт.
                cx.quit();
            }
            Err(error) => self.message = Some((error, true)),
        }
        cx.notify();
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("файл"))
        .to_string_lossy()
        .into_owned()
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = ThemePreference::ALL
            .iter()
            .position(|(theme, _)| *theme == self.theme);
        let border = cx.theme().border;
        let panel = || {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .border_1()
                .border_color(border)
                .rounded_md()
        };
        let heading = |text: &'static str| div().font_weight(FontWeight::BOLD).child(text);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_3()
            .child(
                panel().child(heading("Тема приложения")).child(
                    RadioGroup::horizontal("theme")
                        .selected_index(selected)
                        .children(
                            ThemePreference::ALL
                                .iter()
                                .enumerate()
                                .map(|(index, (_, label))| Radio::new(("theme", index)).label(*label)),
                        )
                        .on_change(cx.listener(|page, index: &usize, window, cx| {
                            if let Some((theme, _)) = ThemePreference::ALL.get(*index) {
                                page.select_theme(*theme, window, cx);
                            }
                        })),
                ),
            )
            .child(
                panel()
                    .child(heading("Резервная копия данных"))
                    .child(
                        "Экспорт и импорт выполняются для всей базы FinanceApp в формате SQLite (.db).",
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("export")
                                    .label("Экспортировать данные…")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|page, _, _, cx| page.start_export(cx))),
                            )
                            .child(
                                Button::new("import")
                                    .label("Импортировать данные…")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|page, _, window, cx| {
                                        page.choose_import_file(window, cx)
                                    })),
                            ),
                    )
                    .when_some(self.operation_message.clone(), |p, text| p.child(text)),
            )
            .when_some(self.message.clone(), |root, (text, is_error)| {
                root.child(div().text_color(if is_error {
                    cx.theme().danger
                } else {
                    cx.theme().success
                })
                .child(text))
            })
    }
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

    #[test]
    fn theme_setting_values_round_trip() {
        for (theme, _) in ThemePreference::ALL {
            assert_eq!(
                ThemePreference::from_setting(theme.setting_value()),
                Some(theme)
            );
        }
    }
}
