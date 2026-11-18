use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    canonical_amount, parse_amount, parse_non_negative_decimal, Bank, InterestDestination,
    SavingsAccount, SavingsDetails, SavingsTerm,
};
use finance_db::{self, Database};

const SAVINGS_PAGE_SIZE: usize = 100;

#[derive(Default)]
pub(super) struct SavingsPage {
    entries: Vec<SavingsAccount>,
    bank_filter: Option<i64>,
    active_filter: Option<bool>,
    search: String,
    current_page: usize,
    form: SavingsForm,
    show_form: bool,
    pending_delete: Option<(i64, String)>,
}

#[derive(Default)]
struct SavingsForm {
    id: Option<i64>,
    bank_id: Option<i64>,
    title: String,
    amount: String,
    base_rate: String,
    use_threshold: bool,
    rate_after_threshold: String,
    threshold_amount: String,
    term_type: Option<SavingsTerm>,
    term_end_date: String,
    interest_destination: Option<InterestDestination>,
    is_active: bool,
    error: Option<String>,
}

enum TableAction {
    Edit(i64),
    Delete(i64, String),
}

impl SavingsPage {
    pub(super) fn refresh(&mut self, database: &Database) -> Result<(), finance_db::DbError> {
        self.entries =
            finance_db::list_savings(database, self.bank_filter, self.active_filter, &self.search)?;
        self.current_page = 0;
        Ok(())
    }

    pub(super) fn render_footer(&self, ui: &mut egui::Ui) {
        ui.label(
            "Процентный доход здесь не рассчитывается: для этого нужны дата открытия, история остатка и правила начисления банка. Повышенная ставка показана как условие, а не как подтверждённая доходность.",
        );
        ui.colored_label(
            egui::Color32::from_rgb(160, 105, 0),
            "Суммы и ставки сохраняются как введены; валюта и фактические начисления для накоплений пока не определены.",
        );
    }

    pub(super) fn begin_new(&mut self, banks: &[Bank]) {
        self.form = SavingsForm {
            bank_id: banks.first().map(|bank| bank.id),
            amount: String::new(),
            base_rate: "0".to_owned(),
            term_type: Some(SavingsTerm::Unlimited),
            interest_destination: Some(InterestDestination::Reinvest),
            is_active: true,
            ..SavingsForm::default()
        };
        self.show_form = true;
    }

    pub(super) fn render(
        &mut self,
        context: &egui::Context,
        ui: &mut egui::Ui,
        database: &Database,
        banks: &[Bank],
    ) -> Option<(String, bool)> {
        if banks.is_empty() {
            ui.label("Сначала добавьте банк в разделе «Справочники», чтобы создать накопление.");
        }

        let mut should_refresh = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Банк");
            let selected_bank = self
                .bank_filter
                .and_then(|id| banks.iter().find(|bank| bank.id == id))
                .map_or("Все банки", |bank| bank.name.as_str());
            egui::ComboBox::from_id_salt("savings_bank_filter")
                .selected_text(selected_bank)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.bank_filter, None, "Все банки");
                    for bank in banks {
                        ui.selectable_value(&mut self.bank_filter, Some(bank.id), &bank.name);
                    }
                });

            ui.label("Статус");
            egui::ComboBox::from_id_salt("savings_active_filter")
                .selected_text(match self.active_filter {
                    None => "Все",
                    Some(true) => "Активные",
                    Some(false) => "Неактивные",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.active_filter, None, "Все");
                    ui.selectable_value(&mut self.active_filter, Some(true), "Активные");
                    ui.selectable_value(&mut self.active_filter, Some(false), "Неактивные");
                });

            ui.label("Поиск");
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Название или банк")
                    .desired_width(180.0),
            );
            if ui.button("Применить фильтры").clicked() {
                should_refresh = true;
            }
            if ui.button("Сбросить").clicked() {
                self.bank_filter = None;
                self.active_filter = None;
                self.search.clear();
                should_refresh = true;
            }
        });

        if should_refresh {
            if let Err(error) = self.refresh(database) {
                return Some((error.to_string(), true));
            }
        }

        if self.entries.is_empty() {
            ui.label("Накоплений по выбранным условиям нет.");
        } else {
            let page_count = self.entries.len().div_ceil(SAVINGS_PAGE_SIZE);
            self.current_page = self.current_page.min(page_count.saturating_sub(1));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.current_page > 0, egui::Button::new("← Назад"))
                    .clicked()
                {
                    self.current_page -= 1;
                }
                if ui
                    .add_enabled(
                        self.current_page + 1 < page_count,
                        egui::Button::new("Далее →"),
                    )
                    .clicked()
                {
                    self.current_page += 1;
                }
                ui.label(format!(
                    "Страница {} из {} · всего записей: {}",
                    self.current_page + 1,
                    page_count,
                    self.entries.len()
                ));
            });
            let mut action = None;
            egui::ScrollArea::both()
                .id_salt("savings_table_scroll")
                .show(ui, |ui| {
                    egui::Grid::new("savings_table")
                        .striped(true)
                        .min_col_width(75.0)
                        .show(ui, |ui| {
                            for heading in [
                                "Банк",
                                "Название",
                                "Сумма",
                                "Базовая ставка",
                                "Ставка при условии",
                                "Порог",
                                "Срок",
                                "Назначение процентов",
                                "Статус",
                                "Действия",
                            ] {
                                ui.strong(heading);
                            }
                            ui.end_row();

                            for account in self
                                .entries
                                .iter()
                                .skip(self.current_page * SAVINGS_PAGE_SIZE)
                                .take(SAVINGS_PAGE_SIZE)
                            {
                                ui.label(&account.bank_name);
                                ui.label(&account.title);
                                ui.label(format!("{}", account.amount));
                                ui.label(format!("{}%", account.base_rate));
                                ui.label(
                                    account
                                        .rate_after_threshold
                                        .map_or_else(|| "—".to_owned(), |rate| format!("{rate}%")),
                                );
                                ui.label(
                                    account.threshold_amount.map_or_else(
                                        || "—".to_owned(),
                                        |amount| format!("{amount}"),
                                    ),
                                );
                                ui.label(match (account.term_type, account.term_end_date) {
                                    (SavingsTerm::Limited, Some(date)) => {
                                        format!("до {}", date.format("%d.%m.%Y"))
                                    }
                                    (SavingsTerm::Limited, None) => "Дата не задана".to_owned(),
                                    (SavingsTerm::Unlimited, _) => "Бессрочный".to_owned(),
                                });
                                ui.label(account.interest_destination.title());
                                ui.label(if account.is_active {
                                    "Активно"
                                } else {
                                    "Неактивно"
                                });
                                ui.horizontal(|ui| {
                                    if ui.small_button("Изменить").clicked() {
                                        action = Some(TableAction::Edit(account.id));
                                    }
                                    if ui.small_button("Удалить").clicked() {
                                        action = Some(TableAction::Delete(
                                            account.id,
                                            account.title.clone(),
                                        ));
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });

            match action {
                Some(TableAction::Edit(id)) => self.begin_edit(id),
                Some(TableAction::Delete(id, title)) => {
                    self.pending_delete = Some((id, title));
                }
                None => {}
            }
        }

        if self.show_form {
            if let Some(result) = self.render_form(ui, database, banks) {
                return Some(result);
            }
        }

        self.render_delete_confirmation(context, database)
    }

    fn begin_edit(&mut self, id: i64) {
        let Some(account) = self.entries.iter().find(|account| account.id == id) else {
            return;
        };
        self.form = SavingsForm {
            id: Some(account.id),
            bank_id: Some(account.bank_id),
            title: account.title.clone(),
            amount: canonical_amount(account.amount),
            base_rate: canonical_amount(account.base_rate),
            use_threshold: account.rate_after_threshold.is_some(),
            rate_after_threshold: account
                .rate_after_threshold
                .map_or_else(String::new, canonical_amount),
            threshold_amount: account
                .threshold_amount
                .map_or_else(String::new, canonical_amount),
            term_type: Some(account.term_type),
            term_end_date: account
                .term_end_date
                .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string()),
            interest_destination: Some(account.interest_destination),
            is_active: account.is_active,
            error: None,
        };
        self.show_form = true;
    }

    fn render_form(
        &mut self,
        ui: &mut egui::Ui,
        database: &Database,
        banks: &[Bank],
    ) -> Option<(String, bool)> {
        ui.separator();
        ui.heading(if self.form.id.is_some() {
            "Изменить накопление"
        } else {
            "Новое накопление"
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Название");
            ui.add(egui::TextEdit::singleline(&mut self.form.title).desired_width(180.0));
            ui.label("Банк");
            let selected_bank = self
                .form
                .bank_id
                .and_then(|id| banks.iter().find(|bank| bank.id == id))
                .map_or("Выберите банк", |bank| bank.name.as_str());
            egui::ComboBox::from_id_salt("savings_form_bank")
                .selected_text(selected_bank)
                .show_ui(ui, |ui| {
                    for bank in banks {
                        ui.selectable_value(&mut self.form.bank_id, Some(bank.id), &bank.name);
                    }
                });
            ui.checkbox(&mut self.form.is_active, "Активно");
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Сумма");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.amount)
                    .desired_width(130.0)
                    .hint_text("Например, 100000"),
            );
            ui.label("Базовая ставка, %");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.base_rate)
                    .desired_width(90.0)
                    .hint_text("Например, 5,5"),
            );
        });

        ui.checkbox(
            &mut self.form.use_threshold,
            "Есть ставка при выполнении условий",
        );
        if self.form.use_threshold {
            ui.horizontal_wrapped(|ui| {
                ui.label("Ставка при условии, %");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.rate_after_threshold)
                        .desired_width(90.0),
                );
                ui.label("Пороговая сумма трат");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.threshold_amount)
                        .desired_width(130.0),
                );
            });
        }

        ui.horizontal_wrapped(|ui| {
            ui.label("Срок");
            egui::ComboBox::from_id_salt("savings_form_term")
                .selected_text(
                    self.form
                        .term_type
                        .map_or("Выберите срок", SavingsTerm::title),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.form.term_type,
                        Some(SavingsTerm::Limited),
                        SavingsTerm::Limited.title(),
                    );
                    ui.selectable_value(
                        &mut self.form.term_type,
                        Some(SavingsTerm::Unlimited),
                        SavingsTerm::Unlimited.title(),
                    );
                });
            if self.form.term_type == Some(SavingsTerm::Limited) {
                ui.label("Дата окончания (ГГГГ-ММ-ДД)");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.term_end_date).desired_width(130.0),
                );
            }

            ui.label("Проценты");
            egui::ComboBox::from_id_salt("savings_interest_destination")
                .selected_text(
                    self.form
                        .interest_destination
                        .map_or("Выберите назначение", InterestDestination::title),
                )
                .show_ui(ui, |ui| {
                    for destination in InterestDestination::ALL {
                        ui.selectable_value(
                            &mut self.form.interest_destination,
                            Some(destination),
                            destination.title(),
                        );
                    }
                });
        });

        if let Some(error) = &self.form.error {
            ui.colored_label(egui::Color32::RED, error);
        }

        let mut save_clicked = false;
        let mut cancel_clicked = false;
        ui.horizontal(|ui| {
            save_clicked = ui.button("Сохранить накопление").clicked();
            cancel_clicked = ui.button("Отмена").clicked();
        });
        if cancel_clicked {
            self.show_form = false;
            return None;
        }
        if !save_clicked {
            return None;
        }

        let details = match self.form.details() {
            Ok(details) => details,
            Err(error) => {
                self.form.error = Some(error);
                return None;
            }
        };
        match finance_db::save_savings(database, self.form.id, &details) {
            Ok(_) => {
                self.show_form = false;
                match self.refresh(database) {
                    Ok(()) => Some(("Накопление сохранено".to_owned(), false)),
                    Err(error) => Some((error.to_string(), true)),
                }
            }
            Err(error) => {
                self.form.error = Some(error.to_string());
                None
            }
        }
    }

    fn render_delete_confirmation(
        &mut self,
        context: &egui::Context,
        database: &Database,
    ) -> Option<(String, bool)> {
        let (id, title) = self.pending_delete.clone()?;
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Подтверждение удаления накопления")
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(format!("Удалить накопление «{title}»?"));
                ui.horizontal(|ui| {
                    confirmed = ui.button("Удалить").clicked();
                    cancelled = ui.button("Отмена").clicked();
                });
            });
        if cancelled {
            self.pending_delete = None;
            return None;
        }
        if confirmed {
            self.pending_delete = None;
            return match finance_db::delete_savings(database, id) {
                Ok(()) => match self.refresh(database) {
                    Ok(()) => Some(("Накопление удалено".to_owned(), false)),
                    Err(error) => Some((error.to_string(), true)),
                },
                Err(error) => Some((error.to_string(), true)),
            };
        }
        None
    }
}

impl SavingsForm {
    fn details(&self) -> Result<SavingsDetails, String> {
        let bank_id = self.bank_id.ok_or_else(|| "Выберите банк".to_owned())?;
        let amount = parse_amount(&self.amount).map_err(|error| error.to_string())?;
        let base_rate = parse_non_negative_decimal(&self.base_rate)
            .map_err(|_| "Введите корректную неотрицательную базовую ставку".to_owned())?;
        let (rate_after_threshold, threshold_amount) = if self.use_threshold {
            (
                Some(
                    parse_non_negative_decimal(&self.rate_after_threshold).map_err(|_| {
                        "Введите корректную неотрицательную ставку при выполнении условия"
                            .to_owned()
                    })?,
                ),
                Some(
                    parse_amount(&self.threshold_amount)
                        .map_err(|error| format!("Пороговая сумма: {error}"))?,
                ),
            )
        } else {
            (None, None)
        };
        let term_type = self
            .term_type
            .ok_or_else(|| "Выберите тип срока".to_owned())?;
        let term_end_date = match term_type {
            SavingsTerm::Limited => Some(
                NaiveDate::parse_from_str(self.term_end_date.trim(), "%Y-%m-%d")
                    .map_err(|_| "Введите дату окончания в формате ГГГГ-ММ-ДД".to_owned())?,
            ),
            SavingsTerm::Unlimited => None,
        };
        let interest_destination = self
            .interest_destination
            .ok_or_else(|| "Выберите назначение процентов".to_owned())?;
        let details = SavingsDetails {
            bank_id,
            title: self.title.clone(),
            amount,
            base_rate,
            rate_after_threshold,
            threshold_amount,
            term_type,
            term_end_date,
            interest_destination,
            is_active: self.is_active,
        };
        details.validate().map_err(|error| error.to_string())?;
        Ok(details)
    }
}
