use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    canonical_amount, parse_amount, parse_non_negative_decimal, Bank, Currency, PaymentCardType,
    PaymentDocument, PaymentDocumentDetails, PaymentSystem, SavingsAccount,
};
use finance_db::{self, Database};

const PAGE_SIZE: usize = 100;

#[derive(Default)]
pub(super) struct PaymentDocumentsPage {
    entries: Vec<PaymentDocument>,
    bank_filter: Option<i64>,
    card_type_filter: Option<PaymentCardType>,
    payment_system_filter: Option<PaymentSystem>,
    current_page: usize,
    form: PaymentDocumentForm,
    available_savings: Vec<SavingsAccount>,
    show_form: bool,
    pending_delete: Option<(i64, String)>,
}

#[derive(Default)]
struct PaymentDocumentForm {
    id: Option<i64>,
    bank_id: Option<i64>,
    payment_system: Option<PaymentSystem>,
    card_type: Option<PaymentCardType>,
    amount: String,
    currency: Option<Currency>,
    due_date: String,
    grace_period_days: String,
    affects_deposit_rate: bool,
    min_monthly_spend: String,
    savings_ids: Vec<i64>,
    error: Option<String>,
}

impl PaymentDocumentsPage {
    pub(super) fn refresh(&mut self, database: &Database) -> Result<(), finance_db::DbError> {
        self.entries = finance_db::list_payment_documents(
            database,
            self.bank_filter,
            self.card_type_filter,
            self.payment_system_filter,
        )?;
        self.current_page = 0;
        Ok(())
    }

    pub(super) fn begin_new(
        &mut self,
        database: &Database,
        bank_id: Option<i64>,
    ) -> Result<(), finance_db::DbError> {
        self.form = PaymentDocumentForm {
            bank_id,
            payment_system: Some(PaymentSystem::Visa),
            card_type: Some(PaymentCardType::Debit),
            ..PaymentDocumentForm::default()
        };
        self.load_savings(database, bank_id)?;
        self.show_form = true;
        Ok(())
    }

    pub(super) fn render(
        &mut self,
        context: &egui::Context,
        ui: &mut egui::Ui,
        database: &Database,
        banks: &[Bank],
    ) -> Option<(String, bool)> {
        if banks.is_empty() {
            ui.label(
                "Сначала добавьте банк в разделе «Справочники», чтобы создать платёжный документ.",
            );
        }

        let mut should_refresh = false;
        egui::ScrollArea::horizontal()
            .id_salt("payment_documents_filters_scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Банк");
                    let selected_bank = self
                        .bank_filter
                        .and_then(|id| banks.iter().find(|bank| bank.id == id))
                        .map_or("Все банки", |bank| bank.name.as_str());
                    egui::ComboBox::from_id_salt("payment_documents_bank_filter")
                        .selected_text(selected_bank)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.bank_filter, None, "Все банки");
                            for bank in banks {
                                ui.selectable_value(
                                    &mut self.bank_filter,
                                    Some(bank.id),
                                    &bank.name,
                                );
                            }
                        });
                    if ui
                        .add_enabled(self.bank_filter.is_some(), egui::Button::new("×"))
                        .on_hover_text("Очистить банк")
                        .clicked()
                    {
                        self.bank_filter = None;
                        should_refresh = true;
                    }

                    ui.label("Платёжная система");
                    egui::ComboBox::from_id_salt("payment_documents_system_filter")
                        .selected_text(
                            self.payment_system_filter
                                .map_or("Все", PaymentSystem::code),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.payment_system_filter, None, "Все");
                            for system in PaymentSystem::ALL {
                                ui.selectable_value(
                                    &mut self.payment_system_filter,
                                    Some(system),
                                    system.code(),
                                );
                            }
                        });

                    ui.label("Тип карты");
                    egui::ComboBox::from_id_salt("payment_documents_type_filter")
                        .selected_text(self.card_type_filter.map_or("Все", PaymentCardType::title))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.card_type_filter, None, "Все");
                            for card_type in PaymentCardType::ALL {
                                ui.selectable_value(
                                    &mut self.card_type_filter,
                                    Some(card_type),
                                    card_type.title(),
                                );
                            }
                        });

                    if ui.button("Применить").clicked() {
                        should_refresh = true;
                    }
                    if ui
                        .add_enabled(
                            self.bank_filter.is_some()
                                || self.card_type_filter.is_some()
                                || self.payment_system_filter.is_some(),
                            egui::Button::new("Сбросить фильтры"),
                        )
                        .clicked()
                    {
                        self.bank_filter = None;
                        self.card_type_filter = None;
                        self.payment_system_filter = None;
                        should_refresh = true;
                    }
                });
            });

        ui.separator();
        if should_refresh {
            if let Err(error) = self.refresh(database) {
                return Some((error.to_string(), true));
            }
        }

        if self.entries.is_empty() {
            ui.label("Платёжных документов по выбранным условиям нет.");
        } else {
            self.render_table(ui, database);
        }

        if self.show_form {
            if let Some(message) = self.render_form(ui, database, banks) {
                return Some(message);
            }
        }

        self.render_delete_confirmation(context, database)
    }

    fn render_table(&mut self, ui: &mut egui::Ui, database: &Database) {
        let page_count = self.entries.len().div_ceil(PAGE_SIZE);
        self.current_page = self.current_page.min(page_count.saturating_sub(1));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.current_page > 0, egui::Button::new("← Назад"))
                .clicked()
            {
                self.current_page -= 1;
            }
            ui.label(format!(
                "Страница {} из {page_count}",
                self.current_page + 1
            ));
            if ui
                .add_enabled(
                    self.current_page + 1 < page_count,
                    egui::Button::new("Далее →"),
                )
                .clicked()
            {
                self.current_page += 1;
            }
        });

        let start = self.current_page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(self.entries.len());
        let entries = self.entries[start..end].to_vec();
        egui::ScrollArea::horizontal()
            .id_salt("payment_documents_table_scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("payment_documents_table")
                    .striped(true)
                    .min_col_width(95.0)
                    .show(ui, |ui| {
                        for title in [
                            "Действия",
                            "Банк",
                            "Платёжная система",
                            "Тип",
                            "Сумма",
                            "Валюта",
                            "Дата платежа",
                            "Льготный период, дни",
                            "Условие ставки",
                            "Минимум трат",
                            "Связанные накопления",
                        ] {
                            ui.strong(title);
                        }
                        ui.end_row();

                        for document in &entries {
                            let (edit_clicked, delete_clicked) = ui
                                .horizontal(|ui| {
                                    (
                                        ui.small_button("Изменить").clicked(),
                                        ui.small_button("Удалить").clicked(),
                                    )
                                })
                                .inner;
                            ui.label(&document.bank_name);
                            ui.label(document.payment_system.code());
                            ui.label(document.card_type.title());
                            let amount_label = match document.card_type {
                                PaymentCardType::Debit => "Текущий баланс",
                                PaymentCardType::Credit => "Кредитный лимит",
                            };
                            ui.label(format!(
                                "{amount_label}: {}",
                                canonical_amount(document.amount)
                            ));
                            ui.label(
                                document
                                    .currency
                                    .map_or("валюта не указана", Currency::code),
                            );
                            ui.label(document.due_date.map_or_else(String::new, |date| {
                                date.format("%Y-%m-%d").to_string()
                            }));
                            ui.label(
                                document
                                    .grace_period_days
                                    .map_or_else(String::new, |days| days.to_string()),
                            );
                            ui.label(if document.affects_deposit_rate {
                                "Условно повышает"
                            } else {
                                "Нет"
                            });
                            ui.label(
                                document
                                    .min_monthly_spend
                                    .map_or_else(String::new, canonical_amount),
                            );
                            ui.label(
                                document
                                    .linked_savings
                                    .iter()
                                    .map(|savings| {
                                        if savings.is_active {
                                            savings.title.clone()
                                        } else {
                                            format!("{} (неактивно)", savings.title)
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            );
                            ui.end_row();

                            if edit_clicked {
                                if let Err(error) = self.begin_edit(database, document.id) {
                                    self.form.error = Some(error.to_string());
                                    self.show_form = true;
                                }
                            }
                            if delete_clicked {
                                self.pending_delete = Some((
                                    document.id,
                                    format!(
                                        "{} — {} {}, {}",
                                        document.bank_name,
                                        document.payment_system.code(),
                                        document.card_type.title(),
                                        canonical_amount(document.amount)
                                    ),
                                ));
                            }
                        }
                    });
            });
    }

    fn begin_edit(&mut self, database: &Database, id: i64) -> Result<(), finance_db::DbError> {
        let Some(document) = self.entries.iter().find(|document| document.id == id) else {
            return Ok(());
        };
        self.form = PaymentDocumentForm {
            id: Some(document.id),
            bank_id: Some(document.bank_id),
            payment_system: Some(document.payment_system),
            card_type: Some(document.card_type),
            amount: canonical_amount(document.amount),
            currency: document.currency,
            due_date: document
                .due_date
                .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string()),
            grace_period_days: document
                .grace_period_days
                .map_or_else(String::new, |days| days.to_string()),
            affects_deposit_rate: document.affects_deposit_rate,
            min_monthly_spend: document
                .min_monthly_spend
                .map_or_else(String::new, canonical_amount),
            savings_ids: document
                .linked_savings
                .iter()
                .map(|savings| savings.id)
                .collect(),
            error: None,
        };
        self.load_savings(database, self.form.bank_id)?;
        self.show_form = true;
        Ok(())
    }

    fn render_form(
        &mut self,
        ui: &mut egui::Ui,
        database: &Database,
        banks: &[Bank],
    ) -> Option<(String, bool)> {
        ui.separator();
        ui.heading(if self.form.id.is_some() {
            "Изменить платёжный документ"
        } else {
            "Новый платёжный документ"
        });

        let previous_bank_id = self.form.bank_id;
        ui.horizontal_wrapped(|ui| {
            ui.label("Банк");
            let selected_bank = self
                .form
                .bank_id
                .and_then(|id| banks.iter().find(|bank| bank.id == id))
                .map_or("Выберите банк", |bank| bank.name.as_str());
            egui::ComboBox::from_id_salt("payment_documents_form_bank")
                .selected_text(selected_bank)
                .show_ui(ui, |ui| {
                    for bank in banks {
                        ui.selectable_value(&mut self.form.bank_id, Some(bank.id), &bank.name);
                    }
                });

            ui.label("Платёжная система");
            egui::ComboBox::from_id_salt("payment_documents_form_system")
                .selected_text(
                    self.form
                        .payment_system
                        .map_or("Выберите", PaymentSystem::code),
                )
                .show_ui(ui, |ui| {
                    for system in PaymentSystem::ALL {
                        ui.selectable_value(
                            &mut self.form.payment_system,
                            Some(system),
                            system.code(),
                        );
                    }
                });

            ui.label("Тип карты");
            egui::ComboBox::from_id_salt("payment_documents_form_type")
                .selected_text(
                    self.form
                        .card_type
                        .map_or("Выберите", PaymentCardType::title),
                )
                .show_ui(ui, |ui| {
                    for card_type in PaymentCardType::ALL {
                        ui.selectable_value(
                            &mut self.form.card_type,
                            Some(card_type),
                            card_type.title(),
                        );
                    }
                });
        });
        if previous_bank_id != self.form.bank_id {
            self.form.savings_ids.clear();
            if let Err(error) = self.load_savings(database, self.form.bank_id) {
                return Some((error.to_string(), true));
            }
        }

        let card_type = self.form.card_type;
        ui.horizontal_wrapped(|ui| {
            ui.label(match card_type {
                Some(PaymentCardType::Debit) => "Текущий баланс",
                Some(PaymentCardType::Credit) => "Кредитный лимит",
                None => "Сумма карты",
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.form.amount)
                    .desired_width(140.0)
                    .hint_text("Например, 100000"),
            );
            ui.label("Валюта карты");
            egui::ComboBox::from_id_salt("payment_documents_form_currency")
                .selected_text(
                    self.form
                        .currency
                        .map_or("валюта не указана", Currency::code),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.form.currency, None, "валюта не указана");
                    for currency in Currency::ALL {
                        ui.selectable_value(
                            &mut self.form.currency,
                            Some(currency),
                            currency.code(),
                        );
                    }
                });
            if card_type == Some(PaymentCardType::Credit) {
                ui.label("Дата платежа (ГГГГ-ММ-ДД)");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.due_date)
                        .desired_width(120.0)
                        .hint_text("2026-12-01"),
                );
                ui.label("Льготный период, дней");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.grace_period_days)
                        .desired_width(70.0),
                );
            }
        });
        if card_type == Some(PaymentCardType::Debit) {
            self.form.due_date.clear();
            self.form.grace_period_days.clear();
        }

        let rate_enabled_changed = ui
            .checkbox(
                &mut self.form.affects_deposit_rate,
                "Условия карты могут влиять на ставку по накоплениям",
            )
            .changed();
        if rate_enabled_changed && !self.form.affects_deposit_rate {
            self.form.min_monthly_spend.clear();
            self.form.savings_ids.clear();
        }
        if self.form.affects_deposit_rate {
            ui.colored_label(
                egui::Color32::from_rgb(160, 105, 0),
                "Условие сохраняется как возможность; приложение не подтверждает выполнение порога трат.",
            );
            ui.horizontal_wrapped(|ui| {
                ui.label("Минимум трат в месяц");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.min_monthly_spend)
                        .desired_width(140.0)
                        .hint_text("Например, 10000"),
                );
            });
            ui.label("Выберите связанные накопления этого же банка:");
            if self.available_savings.is_empty() {
                ui.label("Нет накоплений этого банка. Добавьте накопление, чтобы задать условие.");
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("payment_documents_savings_options")
                    .max_height(120.0)
                    .show(ui, |ui| {
                        for savings in &self.available_savings {
                            let mut selected = self.form.savings_ids.contains(&savings.id);
                            let label = if savings.is_active {
                                savings.title.clone()
                            } else {
                                format!("{} (неактивно)", savings.title)
                            };
                            if ui.checkbox(&mut selected, label).changed() {
                                if selected {
                                    self.form.savings_ids.push(savings.id);
                                } else {
                                    self.form.savings_ids.retain(|id| *id != savings.id);
                                }
                            }
                        }
                    });
            }
        }

        if let Some(error) = &self.form.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        let mut result = None;
        ui.horizontal(|ui| {
            if ui.button("Сохранить").clicked() {
                match self.save(database) {
                    Ok(()) => {
                        self.show_form = false;
                        self.form.error = None;
                        match self.refresh(database) {
                            Ok(()) => {
                                result = Some(("Платёжный документ сохранён".to_owned(), false));
                            }
                            Err(error) => result = Some((error.to_string(), true)),
                        }
                    }
                    Err(error) => self.form.error = Some(error),
                }
            }
            if ui.button("Отмена").clicked() {
                self.show_form = false;
                self.form.error = None;
            }
        });
        result
    }

    fn save(&mut self, database: &Database) -> Result<(), String> {
        let bank_id = self
            .form
            .bank_id
            .ok_or_else(|| "Выберите банк".to_owned())?;
        let payment_system = self
            .form
            .payment_system
            .ok_or_else(|| "Выберите платёжную систему".to_owned())?;
        let card_type = self
            .form
            .card_type
            .ok_or_else(|| "Выберите тип карты".to_owned())?;
        let amount = match card_type {
            PaymentCardType::Debit => parse_non_negative_decimal(&self.form.amount),
            PaymentCardType::Credit => parse_amount(&self.form.amount),
        }
        .map_err(|error| error.to_string())?;
        let (due_date, grace_period_days) = match card_type {
            PaymentCardType::Debit => (None, None),
            PaymentCardType::Credit => {
                let date = NaiveDate::parse_from_str(self.form.due_date.trim(), "%Y-%m-%d")
                    .map_err(|_| "Введите дату платежа в формате ГГГГ-ММ-ДД".to_owned())?;
                let days = self
                    .form
                    .grace_period_days
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| "Введите целое неотрицательное число дней".to_owned())?;
                if days < 0 {
                    return Err("Льготный период не может быть отрицательным".to_owned());
                }
                (Some(date), Some(days))
            }
        };
        let min_monthly_spend = if self.form.affects_deposit_rate {
            Some(parse_amount(&self.form.min_monthly_spend).map_err(|error| error.to_string())?)
        } else {
            None
        };
        let details = PaymentDocumentDetails {
            bank_id,
            payment_system,
            card_type,
            amount,
            currency: self.form.currency,
            due_date,
            grace_period_days,
            affects_deposit_rate: self.form.affects_deposit_rate,
            min_monthly_spend,
            savings_ids: self.form.savings_ids.clone(),
        };
        finance_db::save_payment_document(database, self.form.id, &details)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn load_savings(
        &mut self,
        database: &Database,
        bank_id: Option<i64>,
    ) -> Result<(), finance_db::DbError> {
        self.available_savings = match bank_id {
            Some(bank_id) => finance_db::list_savings(database, Some(bank_id), None, "")?,
            None => Vec::new(),
        };
        Ok(())
    }

    fn render_delete_confirmation(
        &mut self,
        context: &egui::Context,
        database: &Database,
    ) -> Option<(String, bool)> {
        let (id, bank_name) = self.pending_delete.clone()?;
        let mut result = None;
        egui::Window::new("Подтверждение удаления")
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(format!(
                    "Удалить платёжный документ «{bank_name}»? Его связи с накоплениями также будут удалены."
                ));
                ui.horizontal(|ui| {
                    if ui.button("Удалить").clicked() {
                        match finance_db::delete_payment_document(database, id) {
                            Ok(()) => {
                                self.pending_delete = None;
                                match self.refresh(database) {
                                    Ok(()) => {
                                        result = Some((
                                            "Платёжный документ удалён".to_owned(),
                                            false,
                                        ));
                                    }
                                    Err(error) => result = Some((error.to_string(), true)),
                                }
                            }
                            Err(error) => result = Some((error.to_string(), true)),
                        }
                    }
                    if ui.button("Отмена").clicked() {
                        self.pending_delete = None;
                    }
                });
            });
        result
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui;
    use finance_core::{Currency, PaymentCardType};

    use super::PaymentDocumentsPage;

    fn database_with_bank() -> finance_db::Database {
        let database = finance_db::Database::open_in_memory().unwrap();
        database
            .execute_batch(include_str!("../../db/migrations/V1__initial.sql"))
            .unwrap();
        database
            .execute_batch(include_str!(
                "../../db/migrations/V2__transactions_and_directories.sql"
            ))
            .unwrap();
        database
            .execute_batch(include_str!("../../db/migrations/V3__savings.sql"))
            .unwrap();
        database
            .execute_batch(include_str!(
                "../../db/migrations/V4__payment_documents.sql"
            ))
            .unwrap();
        database
            .execute_batch(include_str!("../../db/migrations/V5__savings_dates.sql"))
            .unwrap();
        database
            .execute_batch(include_str!(
                "../../db/migrations/V6__income_card_balances.sql"
            ))
            .unwrap();
        database
            .execute_batch(include_str!("../../db/migrations/V7__expense_cards.sql"))
            .unwrap();
        database
            .execute_batch("INSERT INTO banks (id, name) VALUES (1, 'Банк Альфа');")
            .unwrap();
        database
    }

    #[test]
    fn payment_documents_table_renders_at_minimum_window_size() {
        let database = database_with_bank();
        database
            .execute_batch(
                "INSERT INTO payment_documents (
                     id, bank_id, payment_system, card_type, amount, due_date,
                     grace_period_days, affects_deposit_rate, min_monthly_spend
                 ) VALUES (1, 1, 'MasterCard', 'credit', '50000', '2026-12-01', 55, 0, NULL);
                 INSERT INTO payment_documents (
                     id, bank_id, payment_system, card_type, amount, currency
                 ) VALUES (2, 1, 'VISA', 'debit', '10', 'USD');",
            )
            .unwrap();
        let banks = finance_db::list_banks(&database).unwrap();
        let mut page = PaymentDocumentsPage::default();
        page.refresh(&database).unwrap();
        let context = egui::Context::default();
        let mut message = None;
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1_280.0, 720.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        message = page.render(context, ui, &database, &banks);
                    });
                },
            ));
        }

        assert!(message.is_none());
        let output = output.expect("payment documents page should render twice");
        let rendered_text = output
            .shapes
            .iter()
            .filter_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) => Some(text_shape.galley.job.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(rendered_text.contains(&"Банк Альфа"));
        assert!(rendered_text.contains(&"Кредитный лимит: 50000"));
        assert!(rendered_text.contains(&"валюта не указана"));
        assert!(rendered_text.contains(&"USD"));
        assert!(rendered_text.contains(&"Платёжная система"));
        assert!(rendered_text.contains(&"Удалить"));
    }

    #[test]
    fn new_debit_card_saves_zero_balance_without_automatic_currency() {
        let database = database_with_bank();
        let mut page = PaymentDocumentsPage::default();
        page.begin_new(&database, Some(1)).unwrap();
        assert_eq!(page.form.currency, None);
        page.form.amount = "0".to_owned();

        page.save(&database).unwrap();
        page.refresh(&database).unwrap();
        assert_eq!(page.entries[0].currency, None);
        assert_eq!(page.entries[0].amount.to_string(), "0");
        page.begin_edit(&database, page.entries[0].id).unwrap();
        assert_eq!(page.form.currency, None);
    }

    #[test]
    fn editing_card_preserves_changes_and_clears_currency() {
        let database = database_with_bank();
        let mut page = PaymentDocumentsPage::default();
        page.begin_new(&database, Some(1)).unwrap();
        page.form.amount = "10".to_owned();
        page.form.currency = Some(Currency::Rub);
        page.save(&database).unwrap();
        page.refresh(&database).unwrap();
        let id = page.entries[0].id;

        page.begin_edit(&database, id).unwrap();
        assert_eq!(page.form.currency, Some(Currency::Rub));
        page.form.currency = Some(Currency::Usd);
        page.save(&database).unwrap();
        page.refresh(&database).unwrap();
        page.begin_edit(&database, id).unwrap();
        assert_eq!(page.form.currency, Some(Currency::Usd));
        page.form.currency = None;
        page.save(&database).unwrap();
        page.refresh(&database).unwrap();
        assert_eq!(page.entries[0].currency, None);
    }

    #[test]
    fn negative_debit_balance_and_zero_credit_limit_are_rejected() {
        let database = database_with_bank();
        let mut page = PaymentDocumentsPage::default();
        page.begin_new(&database, Some(1)).unwrap();
        page.form.amount = "-1".to_owned();
        assert!(page.save(&database).is_err());

        page.form.card_type = Some(PaymentCardType::Credit);
        page.form.amount = "0".to_owned();
        page.form.due_date = "2026-12-01".to_owned();
        page.form.grace_period_days = "55".to_owned();
        assert!(page.save(&database).is_err());
        page.refresh(&database).unwrap();
        assert!(page.entries.is_empty());
    }
}
