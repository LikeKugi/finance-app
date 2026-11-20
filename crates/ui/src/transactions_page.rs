use std::collections::HashSet;

use super::category_tree::category_tree_labels;
use super::common::{filter_group, filter_layout, render_filter_actions, TABLE_ROW_HEIGHT};
use super::date_input::date_input;
use super::{DeleteTarget, FinanceApp, TransactionForm, TRANSACTION_TABLE_SCROLL_AREA_ID};
use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    parse_amount_expression, summarize_transaction_balance, Currency, FinancialTransaction,
    PaymentCardType, PaymentDocument, TransactionKind,
};

pub(super) const TRANSACTION_COLUMN_MIN_WIDTHS: [f32; 6] =
    [115.0, 135.0, 90.0, 180.0, 220.0, 180.0];
const TRANSACTION_COLUMN_GROWTH_WEIGHTS: [f32; 6] = [0.8, 0.8, 0.6, 1.1, 1.8, 1.0];
pub(super) const DIVISION_ROUNDING_WARNING: &str = "Деление округляется до 2 знаков после запятой; погрешность может накапливаться при нескольких делениях.";

pub(super) fn transaction_column_widths(available_width: f32) -> [f32; 6] {
    let minimum_width: f32 = TRANSACTION_COLUMN_MIN_WIDTHS.iter().sum();
    let extra_width = (available_width - minimum_width).max(0.0);
    let total_growth_weight: f32 = TRANSACTION_COLUMN_GROWTH_WEIGHTS.iter().sum();

    std::array::from_fn(|index| {
        TRANSACTION_COLUMN_MIN_WIDTHS[index]
            + extra_width * TRANSACTION_COLUMN_GROWTH_WEIGHTS[index] / total_growth_weight
    })
}

fn payment_document_label(card: &PaymentDocument) -> String {
    format!(
        "{} — {} #{} ({})",
        card.bank_name,
        card.payment_system.code(),
        card.id,
        card.currency.map_or("валюта не указана", Currency::code)
    )
}

fn parse_optional_date(value: &str) -> Result<Option<NaiveDate>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map(Some)
        .map_err(|_| "Введите дату фильтра в формате ГГГГ-ММ-ДД".to_owned())
}

pub(super) fn render(app: &mut FinanceApp, ui: &mut egui::Ui, kind: TransactionKind) {
    app.render_transactions(ui, kind);
}

impl FinanceApp {
    pub(super) fn clear_category_filter(&mut self, kind: TransactionKind) {
        self.category_filter = None;
        self.refresh_transactions(kind);
    }

    pub(super) fn clear_currency_filter(&mut self, kind: TransactionKind) {
        self.currency_filter = None;
        self.refresh_transactions(kind);
    }

    pub(super) fn clear_from_date_filter(&mut self, kind: TransactionKind) {
        self.from_date.clear();
        self.from_date_input.reset();
        self.refresh_transactions(kind);
    }

    pub(super) fn clear_search_filter(&mut self, kind: TransactionKind) {
        self.search.clear();
        self.refresh_transactions(kind);
    }

    pub(super) fn clear_to_date_filter(&mut self, kind: TransactionKind) {
        self.to_date.clear();
        self.to_date_input.reset();
        self.refresh_transactions(kind);
    }

    pub(super) fn edit_transaction(&mut self, transaction: &FinancialTransaction) {
        if self
            .active_tab
            .transaction_kind()
            .is_some_and(|kind| !self.load_transaction_cards(kind))
        {
            return;
        }
        self.transaction_date_input.reset();
        self.transaction_form = TransactionForm {
            id: Some(transaction.id),
            date: transaction.date.format("%Y-%m-%d").to_string(),
            category_id: Some(transaction.category_id),
            amount: transaction.amount.to_string(),
            currency: Some(transaction.currency),
            comment: transaction.comment.clone(),
            payment_document_id: transaction.payment_document_id,
            affects_deposit_rate: transaction.affects_deposit_rate,
        };
        self.show_transaction_form = true;
    }

    pub(super) fn load_transaction_cards(&mut self, kind: TransactionKind) -> bool {
        match finance_db::list_payment_documents(
            &self.database,
            None,
            (kind == TransactionKind::Income).then_some(PaymentCardType::Debit),
            None,
        ) {
            Ok(cards) => {
                self.transaction_cards = cards;
                true
            }
            Err(error) => {
                self.show_error(error.to_string());
                false
            }
        }
    }

    pub(super) fn refresh_transactions(&mut self, kind: TransactionKind) {
        let from = match parse_optional_date(&self.from_date) {
            Ok(date) => date,
            Err(message) => {
                self.message = Some((message, true));
                return;
            }
        };
        let to = match parse_optional_date(&self.to_date) {
            Ok(date) => date,
            Err(message) => {
                self.message = Some((message, true));
                return;
            }
        };
        if matches!((from, to), (Some(start), Some(end)) if start > end) {
            self.message = Some((
                "Начало периода не может быть позже окончания".to_owned(),
                true,
            ));
            return;
        }
        let currencies_loaded = self.refresh_currencies();
        let transactions = match finance_db::list_transactions(
            &self.database,
            kind,
            from,
            to,
            self.category_filter,
            self.currency_filter,
            &self.search,
        ) {
            Ok(transactions) => transactions,
            Err(error) => {
                self.transaction_balances = None;
                self.transaction_balance_period = None;
                self.show_error(error.to_string());
                return;
            }
        };
        self.transactions = transactions;

        let amounts = match finance_db::list_transaction_amounts(&self.database, from, to) {
            Ok(amounts) => amounts,
            Err(error) => {
                self.transaction_balances = None;
                self.transaction_balance_period = None;
                self.show_error(error.to_string());
                return;
            }
        };
        match summarize_transaction_balance(&amounts) {
            Ok(balances) => {
                self.transaction_balances = Some(balances);
                self.transaction_balance_period = Some((from, to));
                if currencies_loaded {
                    self.message = None;
                }
            }
            Err(error) => {
                self.transaction_balances = None;
                self.transaction_balance_period = None;
                self.show_error(error.to_string());
            }
        }
    }

    pub(super) fn render_transaction_form(&mut self, ui: &mut egui::Ui, kind: TransactionKind) {
        ui.separator();
        ui.heading(if self.transaction_form.id.is_some() {
            "Изменить операцию"
        } else {
            "Новая операция"
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.label("Дата");
            date_input(
                ui,
                "transaction_date",
                &mut self.transaction_form.date,
                &mut self.transaction_date_input,
                95.0,
            );
            ui.label("Сумма");
            ui.add(
                egui::TextEdit::singleline(&mut self.transaction_form.amount)
                    .desired_width(180.0)
                    .hint_text("например: 100 + 20 - 5 * 2 / 4"),
            );
            ui.label("Валюта");
            let selected_currency = self
                .transaction_form
                .currency
                .map_or("Выберите", Currency::code);
            egui::ComboBox::from_id_salt("transaction_currency")
                .selected_text(selected_currency)
                .show_ui(ui, |ui| {
                    for currency in Currency::ALL {
                        ui.selectable_value(
                            &mut self.transaction_form.currency,
                            Some(currency),
                            currency.code(),
                        );
                    }
                });
            ui.label("Категория");
            let category_labels = category_tree_labels(&self.categories);
            let selected_category = self
                .transaction_form
                .category_id
                .and_then(|id| {
                    category_labels
                        .iter()
                        .find(|(category_id, _)| *category_id == id)
                        .map(|(_, label)| label.as_str())
                })
                .unwrap_or("Выберите");
            let selectable_category_ids: HashSet<_> = self
                .categories
                .iter()
                .filter(|category| {
                    category.is_active || Some(category.id) == self.transaction_form.category_id
                })
                .map(|category| category.id)
                .collect();
            egui::ComboBox::from_id_salt("transaction_category_form")
                .selected_text(selected_category)
                .show_ui(ui, |ui| {
                    for (category_id, label) in &category_labels {
                        if selectable_category_ids.contains(category_id) {
                            ui.selectable_value(
                                &mut self.transaction_form.category_id,
                                Some(*category_id),
                                label,
                            );
                        }
                    }
                });
        });
        ui.add_space(8.0);
        if kind == TransactionKind::Income {
            ui.horizontal(|ui| {
                ui.label("Зачислить на карту");
                let selected = self
                    .transaction_cards
                    .iter()
                    .find(|card| Some(card.id) == self.transaction_form.payment_document_id)
                    .map(payment_document_label)
                    .unwrap_or_else(|| "Без карты".to_owned());
                egui::ComboBox::from_id_salt("income_card")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.transaction_form.payment_document_id,
                            None,
                            "Без карты",
                        );
                        for card in &self.transaction_cards {
                            if card.currency == self.transaction_form.currency
                                && card.currency.is_some()
                            {
                                ui.selectable_value(
                                    &mut self.transaction_form.payment_document_id,
                                    Some(card.id),
                                    payment_document_label(card),
                                );
                            }
                        }
                    });
                if self.transaction_form.payment_document_id.is_some()
                    && !self.transaction_cards.iter().any(|card| {
                        Some(card.id) == self.transaction_form.payment_document_id
                            && card.currency == self.transaction_form.currency
                            && card.currency.is_some()
                    })
                {
                    ui.colored_label(
                        egui::Color32::RED,
                        "Выберите карту в валюте дохода или отмените выбор карты",
                    );
                }
            });
            ui.weak("Доступны дебетовые карты с указанной валютой. Для ранее созданных карт сначала укажите валюту в разделе «Платёжные карты».");
            ui.add_space(8.0);
        } else {
            ui.horizontal(|ui| {
                ui.label("Оплачено картой");
                let selected = self
                    .transaction_cards
                    .iter()
                    .find(|card| Some(card.id) == self.transaction_form.payment_document_id)
                    .map(payment_document_label)
                    .unwrap_or_else(|| "Без карты".to_owned());
                egui::ComboBox::from_id_salt("expense_card")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.transaction_form.payment_document_id,
                            None,
                            "Без карты",
                        );
                        for card in &self.transaction_cards {
                            if card.currency == self.transaction_form.currency
                                && card.currency.is_some()
                            {
                                ui.selectable_value(
                                    &mut self.transaction_form.payment_document_id,
                                    Some(card.id),
                                    payment_document_label(card),
                                );
                            }
                        }
                    });
                if self.transaction_form.payment_document_id.is_some()
                    && !self.transaction_cards.iter().any(|card| {
                        Some(card.id) == self.transaction_form.payment_document_id
                            && card.currency == self.transaction_form.currency
                            && card.currency.is_some()
                    })
                {
                    ui.colored_label(
                        egui::Color32::RED,
                        "Выберите карту в валюте расхода или отмените выбор карты",
                    );
                }
            });
            let rate_eligible = self.transaction_cards.iter().any(|card| {
                Some(card.id) == self.transaction_form.payment_document_id
                    && card.currency == self.transaction_form.currency
                    && card.currency.is_some()
                    && card.affects_deposit_rate
            });
            ui.add_enabled(
                rate_eligible || self.transaction_form.affects_deposit_rate,
                egui::Checkbox::new(
                    &mut self.transaction_form.affects_deposit_rate,
                    "Учитывать расход при расчёте процентов по вкладам",
                ),
            );
            if !rate_eligible {
                ui.weak("Для учёта выберите карту с совпадающей валютой и включённым влиянием на ставку.");
            }
            ui.weak("Связь с картой и отметка не изменяют её баланс; оценка процентов пока не рассчитывается.");
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.label("Комментарий");
            ui.add(
                egui::TextEdit::multiline(&mut self.transaction_form.comment)
                    .desired_width(480.0)
                    .desired_rows(3),
            );
        });
        if self.transaction_form.amount.contains('/') {
            ui.weak(DIVISION_ROUNDING_WARNING);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if !self.transaction_form.amount.trim().is_empty() {
                match parse_amount_expression(&self.transaction_form.amount) {
                    Ok(total) => {
                        ui.label(format!("Итого: {total}"));
                    }
                    Err(error) => {
                        ui.colored_label(egui::Color32::RED, error.to_string());
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Отмена").clicked() {
                    self.show_transaction_form = false;
                }
                if ui.button("Сохранить").clicked() {
                    self.save_transaction(kind);
                }
            });
        });
    }

    pub(super) fn render_transactions(&mut self, ui: &mut egui::Ui, kind: TransactionKind) {
        let mut apply_filters = false;
        let mut reset_filters = false;
        filter_layout(ui, |ui| {
            filter_group(ui, 260.0, |ui| {
                ui.label("Поиск");
                ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(150.0));
                if ui
                    .add_enabled(!self.search.is_empty(), egui::Button::new("×"))
                    .on_hover_text("Очистить поиск")
                    .clicked()
                {
                    self.clear_search_filter(kind);
                }
            });
            filter_group(ui, 185.0, |ui| {
                ui.label("С");
                date_input(
                    ui,
                    "from_date_filter",
                    &mut self.from_date,
                    &mut self.from_date_input,
                    95.0,
                );
                if ui
                    .add_enabled(!self.from_date.is_empty(), egui::Button::new("×"))
                    .on_hover_text("Очистить начальную дату")
                    .clicked()
                {
                    self.clear_from_date_filter(kind);
                }
            });
            filter_group(ui, 185.0, |ui| {
                ui.label("По");
                date_input(
                    ui,
                    "to_date_filter",
                    &mut self.to_date,
                    &mut self.to_date_input,
                    95.0,
                );
                if ui
                    .add_enabled(!self.to_date.is_empty(), egui::Button::new("×"))
                    .on_hover_text("Очистить конечную дату")
                    .clicked()
                {
                    self.clear_to_date_filter(kind);
                }
            });
            filter_group(ui, 360.0, |ui| {
                ui.label("Категория:");
                let category_labels = category_tree_labels(&self.categories);
                let selected_name = self
                    .category_filter
                    .and_then(|id| {
                        category_labels
                            .iter()
                            .find(|(category_id, _)| *category_id == id)
                            .map(|(_, label)| label.as_str())
                    })
                    .unwrap_or("Все");
                egui::ComboBox::from_id_salt("transaction_category_filter")
                    .width(220.0)
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.category_filter, None, "Все")
                            .clicked()
                        {
                            self.refresh_transactions(kind);
                        }
                        for (category_id, label) in &category_labels {
                            if ui
                                .selectable_value(
                                    &mut self.category_filter,
                                    Some(*category_id),
                                    label,
                                )
                                .clicked()
                            {
                                self.refresh_transactions(kind);
                            }
                        }
                    });
                if ui
                    .add_enabled(self.category_filter.is_some(), egui::Button::new("×"))
                    .on_hover_text("Очистить категорию")
                    .clicked()
                {
                    self.clear_category_filter(kind);
                }
            });
            filter_group(ui, 205.0, |ui| {
                ui.add_space(8.0);
                ui.label("Валюта:");
                egui::ComboBox::from_id_salt("transaction_currency_filter")
                    .width(80.0)
                    .selected_text(self.currency_filter.map(Currency::code).unwrap_or("Все"))
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.currency_filter, None, "Все")
                            .clicked()
                        {
                            self.refresh_transactions(kind);
                        }
                        for currency in Currency::ALL {
                            if ui
                                .selectable_value(
                                    &mut self.currency_filter,
                                    Some(currency),
                                    currency.code(),
                                )
                                .clicked()
                            {
                                self.refresh_transactions(kind);
                            }
                        }
                    });
                if ui
                    .add_enabled(self.currency_filter.is_some(), egui::Button::new("×"))
                    .on_hover_text("Очистить валюту")
                    .clicked()
                {
                    self.clear_currency_filter(kind);
                }
            });
            let has_active_filters = !self.search.is_empty()
                || !self.from_date.is_empty()
                || !self.to_date.is_empty()
                || self.category_filter.is_some()
                || self.currency_filter.is_some();
            (apply_filters, reset_filters) = render_filter_actions(ui, has_active_filters);
        });

        if reset_filters {
            self.reset_transaction_filters(kind);
        } else if apply_filters {
            self.refresh_transactions(kind);
        }
        if !self.categories.iter().any(|category| category.is_active) {
            ui.label("Сначала добавьте категорию в разделе «Справочники».");
        }

        if self.show_transaction_form {
            self.render_transaction_form(ui, kind);
        }

        ui.separator();

        let column_spacing = ui.spacing().item_spacing.x;
        let column_widths = transaction_column_widths(
            (ui.available_width()
                - column_spacing * (TRANSACTION_COLUMN_MIN_WIDTHS.len() - 1) as f32)
                .max(0.0),
        );
        let table_width =
            column_widths.iter().sum::<f32>() + column_spacing * (column_widths.len() - 1) as f32;
        egui::ScrollArea::both()
            .id_salt(TRANSACTION_TABLE_SCROLL_AREA_ID)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(table_width);
                let headings = [
                    "Дата",
                    "Сумма",
                    "Валюта",
                    "Категория",
                    "Комментарий",
                    "Действия",
                ];
                ui.horizontal(|ui| {
                    for (heading, width) in headings.into_iter().zip(column_widths) {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, TABLE_ROW_HEIGHT),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(width);
                                ui.add(
                                    egui::Label::new(egui::RichText::new(heading).strong()).wrap(),
                                );
                            },
                        );
                    }
                });

                let transactions = self.transactions.clone();
                for (row_index, transaction) in transactions.iter().enumerate() {
                    let row = ui.horizontal(|ui| {
                        for (value, width) in [
                            transaction.date.format("%d.%m.%Y").to_string(),
                            transaction.amount.to_string(),
                            transaction.currency.code().to_owned(),
                            transaction.category_name.clone(),
                            transaction.comment.clone(),
                        ]
                        .into_iter()
                        .zip(column_widths)
                        {
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, TABLE_ROW_HEIGHT),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_min_width(width);
                                    ui.add(egui::Label::new(value).wrap());
                                },
                            );
                        }
                        ui.allocate_ui_with_layout(
                            egui::vec2(column_widths[5], TABLE_ROW_HEIGHT),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                if ui.small_button("Изменить").clicked() {
                                    self.edit_transaction(transaction);
                                }
                                if ui.small_button("Удалить").clicked() {
                                    self.pending_delete =
                                        Some(DeleteTarget::Transaction(kind, transaction.id));
                                }
                            },
                        );
                    });
                    if row_index % 2 == 1 {
                        ui.ctx()
                            .layer_painter(egui::LayerId::new(
                                egui::Order::Background,
                                ui.layer_id().id,
                            ))
                            .rect_filled(row.response.rect, 0.0, ui.visuals().faint_bg_color);
                    }
                }
            });
    }

    pub(super) fn reset_transaction_filters(&mut self, kind: TransactionKind) {
        self.search.clear();
        self.from_date.clear();
        self.to_date.clear();
        self.from_date_input.reset();
        self.to_date_input.reset();
        self.category_filter = None;
        self.currency_filter = None;
        self.refresh_transactions(kind);
    }

    pub(super) fn save_transaction(&mut self, kind: TransactionKind) {
        let date = match NaiveDate::parse_from_str(&self.transaction_form.date, "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => {
                self.show_error("Введите дату в формате ГГГГ-ММ-ДД".to_owned());
                return;
            }
        };
        let amount = match parse_amount_expression(&self.transaction_form.amount) {
            Ok(amount) => amount,
            Err(error) => {
                self.show_error(error.to_string());
                return;
            }
        };
        let Some(category_id) = self.transaction_form.category_id else {
            self.show_error("Выберите категорию".to_owned());
            return;
        };
        let Some(currency) = self.transaction_form.currency else {
            self.show_error("Выберите валюту".to_owned());
            return;
        };
        match finance_db::save_transaction_with_card_and_rate_effect(
            &self.database,
            kind,
            self.transaction_form.id,
            date,
            category_id,
            amount,
            currency,
            &self.transaction_form.comment,
            self.transaction_form.payment_document_id,
            self.transaction_form.affects_deposit_rate,
        ) {
            Ok(_) => {
                self.show_transaction_form = false;
                self.show_success("Операция сохранена");
                self.refresh_transactions(kind);
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    pub(super) fn start_new_transaction(&mut self) {
        if self
            .active_tab
            .transaction_kind()
            .is_some_and(|kind| !self.load_transaction_cards(kind))
        {
            return;
        }
        self.transaction_date_input.reset();
        self.transaction_form = TransactionForm {
            date: chrono::Local::now()
                .date_naive()
                .format("%Y-%m-%d")
                .to_string(),
            category_id: self
                .categories
                .iter()
                .find(|category| category.is_active)
                .map(|category| category.id),
            currency: Some(Currency::Rub),
            ..TransactionForm::default()
        };
        self.show_transaction_form = true;
    }
}
