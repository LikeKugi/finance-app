use std::str::FromStr;

use chrono::NaiveDate;
use eframe::egui;
use finance_core::{CurrencyDefinition, TransactionKind};
use rust_decimal::Decimal;

use super::category_tree::category_tree_rows;
use super::{
    BankForm, CategoryForm, CurrencyForm, DeleteTarget, DirectoryRightTab, FinanceApp,
    DIRECTORY_BANKS_SCROLL_AREA_ID, DIRECTORY_CATEGORIES_SCROLL_AREA_ID,
};

pub(super) fn currency_rate_label(currency: &CurrencyDefinition, primary_code: &str) -> String {
    if currency.is_primary {
        return "Основная валюта".to_owned();
    }
    match (currency.rate_to_primary, currency.rate_date) {
        (Some(rate), Some(date)) => format!(
            "1 {} = {} {} · курс от {}",
            currency.code,
            rate,
            primary_code,
            date.format("%d.%m.%Y")
        ),
        _ => "Курс не задан".to_owned(),
    }
}

impl FinanceApp {
    pub(super) fn change_primary_currency(&mut self, code: &str) {
        match finance_db::set_primary_currency(&mut self.database, code) {
            Ok(()) => {
                self.show_success("Основная валюта изменена; суммы операций не пересчитаны");
                self.refresh_directories();
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    pub(super) fn edit_currency_rate(&mut self, currency: &CurrencyDefinition) {
        self.currency_form = CurrencyForm {
            code: currency.code.clone(),
            rate: currency
                .rate_to_primary
                .map_or_else(String::new, |rate| rate.to_string()),
            rate_date: currency.rate_date.map_or_else(
                || {
                    chrono::Local::now()
                        .date_naive()
                        .format("%Y-%m-%d")
                        .to_string()
                },
                |date| date.format("%Y-%m-%d").to_string(),
            ),
            editing_rate: true,
            ..CurrencyForm::default()
        };
        self.show_currency_form = true;
    }

    pub(super) fn render_bank_directory(&mut self, ui: &mut egui::Ui) {
        let banks = self.banks.clone();
        let table_width = ui.available_width();
        let name_width = (table_width - 145.0).max(100.0);
        let row_height = 22.0;
        ui.allocate_ui_with_layout(
            egui::vec2(table_width, row_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(name_width, row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(egui::RichText::new("Банк").strong());
                    },
                );
                ui.strong("Действия");
            },
        );

        for (row_index, bank) in banks.iter().enumerate() {
            let mut edit_clicked = false;
            let mut delete_clicked = false;
            ui.allocate_ui_with_layout(
                egui::vec2(table_width, row_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if row_index % 2 == 1 {
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, ui.visuals().faint_bg_color);
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(name_width, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(&bank.name);
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        delete_clicked = ui.small_button("Удалить").clicked();
                        edit_clicked = ui.small_button("Изменить").clicked();
                    });
                },
            );
            if edit_clicked {
                self.bank_form = BankForm {
                    id: Some(bank.id),
                    name: bank.name.clone(),
                    comment: bank.comment.clone(),
                };
                self.show_bank_form = true;
            }
            if delete_clicked {
                self.pending_delete = Some(DeleteTarget::Bank(bank.id));
            }
        }
    }

    pub(super) fn render_bank_form(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading(if self.bank_form.id.is_some() {
            "Изменить банк"
        } else {
            "Новый банк"
        });
        ui.horizontal(|ui| {
            ui.label("Название");
            ui.text_edit_singleline(&mut self.bank_form.name);
            ui.label("Комментарий");
            ui.text_edit_singleline(&mut self.bank_form.comment);
        });
        ui.horizontal(|ui| {
            if ui.button("Сохранить банк").clicked() {
                self.save_bank();
            }
            if ui.button("Отмена").clicked() {
                self.show_bank_form = false;
            }
        });
    }

    pub(super) fn render_category_form(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading(if self.category_form.id.is_some() {
            "Изменить категорию"
        } else {
            "Новая категория"
        });
        ui.horizontal(|ui| {
            ui.label("Название");
            ui.text_edit_singleline(&mut self.category_form.name);
            ui.label("Родительская категория");
            let selected_parent = self
                .category_form
                .parent_id
                .and_then(|id| self.categories.iter().find(|category| category.id == id))
                .map_or("Нет", |category| category.name.as_str());
            egui::ComboBox::from_id_salt("category_parent")
                .selected_text(selected_parent)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category_form.parent_id, None, "Нет");
                    for category in &self.categories {
                        if Some(category.id) != self.category_form.id {
                            ui.selectable_value(
                                &mut self.category_form.parent_id,
                                Some(category.id),
                                &category.name,
                            );
                        }
                    }
                });
            ui.checkbox(&mut self.category_form.is_active, "Активна");
        });
        ui.horizontal(|ui| {
            if ui.button("Сохранить категорию").clicked() {
                self.save_category();
            }
            if ui.button("Отмена").clicked() {
                self.show_category_form = false;
            }
        });
    }

    pub(super) fn render_currency_directory(&mut self, ui: &mut egui::Ui) {
        let currencies = self.currencies.clone();
        let used_currencies = self.used_currencies.clone();
        let primary_code = currencies
            .iter()
            .find(|currency| currency.is_primary)
            .map_or("—", |currency| currency.code.as_str())
            .to_owned();
        let table_width = ui.available_width();
        let row_height = 44.0;

        ui.label(egui::RichText::new("Валюта и курс к основной").strong());
        for (row_index, currency) in currencies.iter().enumerate() {
            let mut edit_rate_clicked = false;
            let mut primary_clicked = false;
            let mut delete_clicked = false;
            ui.allocate_ui_with_layout(
                egui::vec2(table_width, row_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if row_index % 2 == 1 {
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, ui.visuals().faint_bg_color);
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&currency.code);
                            ui.label(&currency.name);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    delete_clicked = ui
                                        .add_enabled(
                                            !currency.is_primary
                                                && !used_currencies.contains(&currency.code),
                                            egui::Button::new("Удалить").small(),
                                        )
                                        .on_disabled_hover_text(
                                            if currency.is_primary {
                                                "Сначала выберите другую основную валюту"
                                            } else {
                                                "Валюта используется; сначала удалите связанные финансовые записи"
                                            },
                                        )
                                        .clicked();
                                    if currency.is_primary {
                                        ui.weak("Основная");
                                    } else {
                                        primary_clicked = ui
                                            .add_enabled(
                                                currency.rate_to_primary.is_some(),
                                                egui::Button::new("Сделать основной").small(),
                                            )
                                            .on_disabled_hover_text(
                                                "Сначала задайте курс этой валюты",
                                            )
                                            .clicked();
                                    }
                                    edit_rate_clicked = ui
                                        .add_enabled(
                                            !currency.is_primary,
                                            egui::Button::new("Изменить курс").small(),
                                        )
                                        .clicked();
                                },
                            );
                        });
                        ui.weak(currency_rate_label(currency, &primary_code));
                    });
                },
            );
            if edit_rate_clicked {
                self.edit_currency_rate(currency);
            }
            if primary_clicked {
                self.change_primary_currency(&currency.code);
            }
            if delete_clicked {
                self.pending_delete = Some(DeleteTarget::Currency(currency.code.clone()));
            }
        }
        ui.add_space(8.0);
        ui.weak("Курс означает количество единиц основной валюты за 1 единицу выбранной валюты.");
        ui.weak("Дополнительные валюты пока нельзя выбирать в формах доходов и расходов (поддерживаются RUB и USD).");
    }

    pub(super) fn render_currency_form(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        if self.currency_form.editing_rate {
            let primary_code = self
                .currencies
                .iter()
                .find(|currency| currency.is_primary)
                .map_or("—", |currency| currency.code.as_str());
            ui.heading(format!(
                "Курс {} к {}",
                self.currency_form.code, primary_code
            ));
            ui.horizontal(|ui| {
                ui.label("За 1 единицу валюты");
                ui.text_edit_singleline(&mut self.currency_form.rate);
                ui.label("Дата курса (ГГГГ-ММ-ДД)");
                ui.text_edit_singleline(&mut self.currency_form.rate_date);
            });
        } else {
            ui.heading("Новая валюта");
            ui.horizontal(|ui| {
                ui.label("Код (3 заглавные латинские буквы)");
                ui.text_edit_singleline(&mut self.currency_form.code);
                ui.label("Название");
                ui.text_edit_singleline(&mut self.currency_form.name);
            });
        }
        ui.horizontal(|ui| {
            if ui.button("Сохранить").clicked() {
                self.save_currency();
            }
            if ui.button("Отмена").clicked() {
                self.show_currency_form = false;
                self.currency_form = CurrencyForm::default();
            }
        });
    }

    pub(super) fn render_directories(&mut self, ui: &mut egui::Ui) -> (egui::Id, egui::Id) {
        let previous_kind = self.directory_kind;
        let previous_right_tab = self.directory_right_tab;
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.directory_kind,
                TransactionKind::Income,
                "Категории доходов",
            );
            ui.selectable_value(
                &mut self.directory_kind,
                TransactionKind::Expense,
                "Категории расходов",
            );
        });
        if self.directory_kind != previous_kind {
            self.category_form = CategoryForm::default();
            self.show_category_form = false;
            self.refresh_categories(self.directory_kind, true);
        }
        if self.directory_right_tab != previous_right_tab {
            self.show_bank_form = false;
            self.show_currency_form = false;
            self.currency_form = CurrencyForm::default();
        }
        let mut scroll_area_ids = None;
        ui.columns(2, |columns| {
            columns[0].horizontal(|ui| {
                ui.heading("Категории");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("＋ Добавить категорию").clicked() {
                        self.category_form = CategoryForm {
                            is_active: true,
                            ..CategoryForm::default()
                        };
                        self.show_category_form = true;
                    }
                });
            });
            let category_scroll_area = egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .id_salt(DIRECTORY_CATEGORIES_SCROLL_AREA_ID);
            let category_scroll_output = category_scroll_area.show(&mut columns[0], |ui| {
                let categories = self.categories.clone();
                let category_rows = category_tree_rows(&categories);
                let table_width = ui.available_width();
                let name_width = (table_width - 145.0).max(100.0);
                let row_height = 22.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(table_width, row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(name_width, row_height),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.label(egui::RichText::new("Категория").strong());
                            },
                        );
                        ui.strong("Действия");
                    },
                );

                for (row_index, (index, depth)) in category_rows.into_iter().enumerate() {
                    let category = &categories[index];
                    let prefix = if depth == 0 {
                        String::new()
                    } else {
                        format!("{}↳ ", "　".repeat(depth))
                    };
                    let active_suffix = if category.is_active {
                        ""
                    } else {
                        " (неактивна)"
                    };
                    let mut edit_clicked = false;
                    let mut delete_clicked = false;
                    ui.allocate_ui_with_layout(
                        egui::vec2(table_width, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if row_index % 2 == 1 {
                                ui.painter().rect_filled(
                                    ui.max_rect(),
                                    0.0,
                                    ui.visuals().faint_bg_color,
                                );
                            }
                            ui.allocate_ui_with_layout(
                                egui::vec2(name_width, row_height),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(format!("{prefix}{}{active_suffix}", category.name));
                                },
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    delete_clicked = ui.small_button("Удалить").clicked();
                                    edit_clicked = ui.small_button("Изменить").clicked();
                                },
                            );
                        },
                    );
                    if edit_clicked {
                        self.category_form = CategoryForm {
                            id: Some(category.id),
                            name: category.name.clone(),
                            parent_id: category.parent_id,
                            is_active: category.is_active,
                        };
                        self.show_category_form = true;
                    }
                    if delete_clicked {
                        self.pending_delete =
                            Some(DeleteTarget::Category(self.directory_kind, category.id));
                    }
                }
            });
            let right_title = match self.directory_right_tab {
                DirectoryRightTab::Banks => "Банки",
                DirectoryRightTab::Currencies => "Валюты",
            };
            columns[1].horizontal(|ui| {
                ui.heading(right_title);
                ui.selectable_value(
                    &mut self.directory_right_tab,
                    DirectoryRightTab::Banks,
                    "Банки",
                );
                ui.selectable_value(
                    &mut self.directory_right_tab,
                    DirectoryRightTab::Currencies,
                    "Валюты",
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match self.directory_right_tab {
                        DirectoryRightTab::Banks => {
                            if ui.button("＋ Добавить банк").clicked() {
                                self.bank_form = BankForm::default();
                                self.show_bank_form = true;
                            }
                        }
                        DirectoryRightTab::Currencies => {
                            if ui.button("＋ Добавить валюту").clicked() {
                                self.currency_form = CurrencyForm::default();
                                self.show_currency_form = true;
                            }
                        }
                    },
                );
            });
            let right_scroll_area = egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .id_salt(DIRECTORY_BANKS_SCROLL_AREA_ID);
            let right_scroll_output =
                right_scroll_area.show(&mut columns[1], |ui| match self.directory_right_tab {
                    DirectoryRightTab::Banks => {
                        self.render_bank_directory(ui);
                    }
                    DirectoryRightTab::Currencies => {
                        self.render_currency_directory(ui);
                    }
                });
            scroll_area_ids = Some((category_scroll_output.id, right_scroll_output.id));
        });

        if self.directory_right_tab != previous_right_tab {
            self.show_bank_form = false;
            self.show_currency_form = false;
            self.currency_form = CurrencyForm::default();
        }
        if self.show_category_form {
            self.render_category_form(ui);
        }
        if self.show_bank_form {
            self.render_bank_form(ui);
        }
        if self.show_currency_form {
            self.render_currency_form(ui);
        }
        scroll_area_ids.expect("directory columns should render both scroll areas")
    }

    pub(super) fn save_bank(&mut self) {
        match finance_db::save_bank(
            &self.database,
            self.bank_form.id,
            &self.bank_form.name,
            &self.bank_form.comment,
        ) {
            Ok(_) => {
                self.show_bank_form = false;
                self.show_success("Банк сохранён");
                self.refresh_directories();
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    pub(super) fn save_category(&mut self) {
        match finance_db::save_category(
            &self.database,
            self.directory_kind,
            self.category_form.id,
            &self.category_form.name,
            self.category_form.parent_id,
            self.category_form.is_active,
        ) {
            Ok(_) => {
                self.show_category_form = false;
                self.show_success("Категория сохранена");
                self.refresh_directories();
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    pub(super) fn save_currency(&mut self) {
        let result = if self.currency_form.editing_rate {
            let rate_text = self.currency_form.rate.trim().replace(',', ".");
            let rate = match Decimal::from_str(&rate_text) {
                Ok(rate) => rate,
                Err(_) => {
                    self.show_error("Введите корректный положительный курс".to_owned());
                    return;
                }
            };
            let rate_date =
                match NaiveDate::parse_from_str(self.currency_form.rate_date.trim(), "%Y-%m-%d") {
                    Ok(date) => date,
                    Err(_) => {
                        self.show_error("Введите дату курса в формате ГГГГ-ММ-ДД".to_owned());
                        return;
                    }
                };
            finance_db::set_currency_exchange_rate(
                &self.database,
                &self.currency_form.code,
                rate,
                rate_date,
            )
        } else {
            let code = self.currency_form.code.trim().to_ascii_uppercase();
            finance_db::add_currency(&self.database, &code, &self.currency_form.name)
        };

        match result {
            Ok(()) => {
                self.show_currency_form = false;
                self.show_success(if self.currency_form.editing_rate {
                    "Курс валюты сохранён"
                } else {
                    "Валюта добавлена"
                });
                self.currency_form = CurrencyForm::default();
                self.refresh_directories();
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }
}
