//! Страница «Настройки» на `gpui-kit`: тема и резервное копирование всей базы SQLite.
//! Экспорт и подготовка
//! импорта выполняются в фоне через существующие функции `finance-db`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use finance_db::Database;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Theme, ThemeMode, ThemeRegistry, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rfd::FileDialog;

use super::selectable::{selectable_message, selectable_text};

const LIGHT_THEME_NAME: &str = "FinanceApp Light";
const DARK_THEME_NAME: &str = "FinanceApp Dark";
const LIGHT_THEME_CONFIG: &str = include_str!("../../../../assets/themes/financeapp-light.json");
const DARK_THEME_CONFIG: &str = include_str!("../../../../assets/themes/financeapp-dark.json");

pub(crate) fn register_project_themes(cx: &mut App) {
    if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(LIGHT_THEME_CONFIG) {
        tracing::error!("Не удалось загрузить палитру светлой темы FinanceApp: {error}");
    }
    if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(DARK_THEME_CONFIG) {
        tracing::error!("Не удалось загрузить палитру тёмной темы FinanceApp: {error}");
    }
}

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
        let (light_theme, dark_theme) = {
            let themes = ThemeRegistry::global(cx).themes();
            (
                themes.get(LIGHT_THEME_NAME).cloned(),
                themes.get(DARK_THEME_NAME).cloned(),
            )
        };
        let theme = Theme::global_mut(cx);
        if let Some(light_theme) = light_theme {
            theme.light_theme = light_theme;
        }
        if let Some(dark_theme) = dark_theme {
            theme.dark_theme = dark_theme;
        }

        match self {
            Self::System => Theme::sync_system_appearance(Some(window), cx),
            Self::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Self::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }
}

/// Сохранённый выбор полноэкранного запуска и причина безопасного отката к оконному режиму.
pub(crate) struct StartFullscreenPreference {
    pub(crate) enabled: bool,
    pub(crate) error: Option<String>,
}

impl StartFullscreenPreference {
    const SETTING_KEY: &'static str = "start_fullscreen";

    pub(crate) fn load(database: &Database) -> Self {
        match finance_db::get_app_setting(database, Self::SETTING_KEY) {
            Ok(value) => Self::from_setting(value.as_deref()),
            Err(error) => Self {
                enabled: false,
                error: Some(format!(
                    "Не удалось прочитать настройку полноэкранного запуска: {error}"
                )),
            },
        }
    }

    fn from_setting(value: Option<&str>) -> Self {
        match value {
            None => Self {
                enabled: false,
                error: None,
            },
            Some("true") => Self {
                enabled: true,
                error: None,
            },
            Some("false") => Self {
                enabled: false,
                error: None,
            },
            Some(value) => Self {
                enabled: false,
                error: Some(format!(
                    "Некорректная настройка полноэкранного запуска («{value}»); используется оконный режим"
                )),
            },
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
    app_version: &'static str,
    theme: ThemePreference,
    start_fullscreen: bool,
    start_fullscreen_error: Option<String>,
    message: Option<(String, bool)>,
    operation_message: Option<String>,
    busy: bool,
}

impl SettingsPage {
    pub(crate) fn new(
        database: Rc<Database>,
        database_path: PathBuf,
        app_version: &'static str,
        start_fullscreen: StartFullscreenPreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut page = Self {
            database,
            database_path,
            app_version,
            theme: ThemePreference::System,
            start_fullscreen: start_fullscreen.enabled,
            start_fullscreen_error: start_fullscreen.error,
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

    fn select_start_fullscreen(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.start_fullscreen {
            return;
        }
        let value = if enabled { "true" } else { "false" };
        match finance_db::set_app_setting(
            &self.database,
            StartFullscreenPreference::SETTING_KEY,
            value,
        ) {
            Ok(()) => {
                self.start_fullscreen = enabled;
                self.start_fullscreen_error = None;
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
                    .child(heading("Запуск приложения"))
                    .child(
                        Checkbox::new("start-fullscreen")
                            .label("Открывать приложение в полноэкранном режиме")
                            .checked(self.start_fullscreen)
                            .on_click(cx.listener(|page, checked: &bool, _, cx| {
                                page.select_start_fullscreen(*checked, cx);
                            })),
                    )
                    .child(selectable_text(
                        "settings-start-fullscreen-note",
                        "Новый режим окна будет применён при следующем запуске приложения.",
                    ))
                    .when_some(self.start_fullscreen_error.clone(), |p, text| {
                        p.child(selectable_message(
                            "settings-start-fullscreen-error",
                            text,
                            cx.theme().danger,
                        ))
                    }),
            )
            .child(
                panel()
                    .child(heading("Резервная копия данных"))
                    .child(
                        selectable_text(
                            "settings-backup-description",
                            "Экспорт и импорт выполняются для всей базы FinanceApp в формате SQLite (.db).",
                        ),
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
                    .when_some(self.operation_message.clone(), |p, text| {
                        p.child(selectable_text("settings-operation-message", text))
                    }),
            )
            .when_some(self.message.clone(), |root, (text, is_error)| {
                root.child(selectable_message(
                    "settings-message",
                    text,
                    if is_error {
                        cx.theme().danger
                    } else {
                        cx.theme().success
                    },
                ))
            })
            .child(div().flex_1().min_h_0())
            .child(
                div()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(selectable_text(
                        "settings-version",
                        format!("Версия приложения: {}", self.app_version),
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{StartFullscreenPreference, ThemePreference};
    use gpui_kit::component::{ThemeMode, ThemeSet};

    const DARK_THEME_CONFIG: &str = include_str!("../../../../assets/themes/financeapp-dark.json");
    const LIGHT_THEME_CONFIG: &str =
        include_str!("../../../../assets/themes/financeapp-light.json");

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

    #[test]
    fn project_dark_theme_config_contains_palette_and_editor_colors() {
        let value = serde_json::from_str::<serde_json::Value>(DARK_THEME_CONFIG)
            .expect("project dark theme config should be valid JSON");
        let themes = serde_json::from_value::<ThemeSet>(value.clone())
            .expect("project dark theme config should parse");
        let theme = themes
            .themes
            .first()
            .expect("project dark theme config should define a theme");

        assert_eq!(theme.name.as_ref(), "FinanceApp Dark");
        assert_eq!(theme.mode, ThemeMode::Dark);
        assert_eq!(theme.colors.background.as_deref(), Some("#1e1e1e"));
        assert_eq!(theme.colors.foreground.as_deref(), Some("#dcdcee"));
        assert_eq!(
            theme.colors.description_list_label.as_deref(),
            Some("#282829")
        );
        assert_eq!(theme.colors.link.as_deref(), Some("#dcdcee"));
        assert_eq!(theme.colors.drag_border.as_deref(), Some("#457deb"));
        assert_eq!(theme.colors.progress_bar.as_deref(), Some("#d8d8ea"));
        assert_eq!(theme.colors.slider_bar.as_deref(), Some("#dcdcee"));
        assert_eq!(theme.colors.scrollbar.as_deref(), Some("#28282900"));
        assert_eq!(theme.colors.table_row_border.as_deref(), Some("#343436b3"));
        assert!(theme.highlight.is_some());

        let highlight = &value["themes"][0]["highlight"];
        assert_eq!(highlight["editor.foreground"].as_str(), Some("#b6b8c4"));
        assert_eq!(highlight["editor.invisible"].as_str(), Some("#71717966"));
        assert_eq!(
            highlight["syntax"]["string"]["color"].as_str(),
            Some("#97c79f")
        );
    }

    #[test]
    fn project_light_theme_config_contains_palette_and_editor_colors() {
        let value = serde_json::from_str::<serde_json::Value>(LIGHT_THEME_CONFIG)
            .expect("project light theme config should be valid JSON");
        let themes = serde_json::from_value::<ThemeSet>(value.clone())
            .expect("project light theme config should parse");
        let theme = themes
            .themes
            .first()
            .expect("project light theme config should define a theme");

        assert_eq!(theme.name.as_ref(), "FinanceApp Light");
        assert_eq!(theme.mode, ThemeMode::Light);
        assert_eq!(theme.colors.background.as_deref(), Some("#f7f7fa"));
        assert_eq!(theme.colors.foreground.as_deref(), Some("#1e1e1e"));
        assert_eq!(
            theme.colors.description_list_label.as_deref(),
            Some("#eeeef4")
        );
        assert_eq!(theme.colors.link.as_deref(), Some("#2659a0"));
        assert_eq!(theme.colors.drag_border.as_deref(), Some("#3867ba"));
        assert_eq!(theme.colors.progress_bar.as_deref(), Some("#45456a"));
        assert_eq!(theme.colors.slider_bar.as_deref(), Some("#45456a"));
        assert_eq!(theme.colors.scrollbar.as_deref(), Some("#efeff500"));
        assert_eq!(theme.colors.table_row_border.as_deref(), Some("#cbcbdbb3"));
        assert!(theme.highlight.is_some());

        let highlight = &value["themes"][0]["highlight"];
        assert_eq!(highlight["editor.foreground"].as_str(), Some("#1e1e1e"));
        assert_eq!(highlight["editor.invisible"].as_str(), Some("#66667a66"));
        assert_eq!(
            highlight["syntax"]["string"]["color"].as_str(),
            Some("#2b663d")
        );
    }

    #[test]
    fn start_fullscreen_setting_defaults_to_windowed_mode() {
        let preference = StartFullscreenPreference::from_setting(None);
        assert!(!preference.enabled);
        assert!(preference.error.is_none());
    }

    #[test]
    fn start_fullscreen_setting_values_select_the_window_mode() {
        let enabled = StartFullscreenPreference::from_setting(Some("true"));
        assert!(enabled.enabled);
        assert!(enabled.error.is_none());

        let disabled = StartFullscreenPreference::from_setting(Some("false"));
        assert!(!disabled.enabled);
        assert!(disabled.error.is_none());
    }

    #[test]
    fn invalid_start_fullscreen_setting_falls_back_to_windowed_mode() {
        let preference = StartFullscreenPreference::from_setting(Some("unknown"));
        assert!(!preference.enabled);
        let error = preference.error.expect("invalid value should be reported");
        assert!(error.contains("оконный режим"), "unexpected error: {error}");
    }
}
