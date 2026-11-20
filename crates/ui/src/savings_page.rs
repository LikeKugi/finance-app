use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    canonical_amount, parse_amount, parse_non_negative_decimal, Bank, InterestDestination,
    SavingsAccount, SavingsDetails, SavingsTerm,
};
use finance_db::{self, Database};

use super::common::TABLE_ROW_HEIGHT;

const SAVINGS_PAGE_SIZE: usize = 100;
const SAVINGS_COLUMN_MIN_WIDTHS: [f32; 12] = [
    80.0, 100.0, 75.0, 95.0, 105.0, 75.0, 80.0, 90.0, 105.0, 120.0, 75.0, 145.0,
];
const SAVINGS_COLUMN_GROWTH_WEIGHTS: [f32; 12] =
    [0.8, 1.1, 0.9, 1.0, 1.2, 0.9, 0.9, 0.9, 1.0, 1.1, 0.8, 1.0];

fn savings_column_widths(available_width: f32) -> [f32; 12] {
    let minimum_width: f32 = SAVINGS_COLUMN_MIN_WIDTHS.iter().sum();
    let extra_width = (available_width - minimum_width).max(0.0);
    let total_growth_weight: f32 = SAVINGS_COLUMN_GROWTH_WEIGHTS.iter().sum();

    std::array::from_fn(|index| {
        SAVINGS_COLUMN_MIN_WIDTHS[index]
            + extra_width * SAVINGS_COLUMN_GROWTH_WEIGHTS[index] / total_growth_weight
    })
}

#[derive(Default)]
pub(super) struct SavingsPage {
    entries: Vec<SavingsAccount>,
    bank_filter: Option<i64>,
    active_filter: Option<bool>,
    search: String,
    current_page: usize,
    form: SavingsForm,
    form_snapshot: Option<SavingsForm>,
    show_form: bool,
    show_discard_confirmation: bool,
    pending_delete: Option<(i64, String)>,
}

#[derive(Clone, Default, PartialEq)]
struct SavingsForm {
    id: Option<i64>,
    bank_id: Option<i64>,
    title: String,
    amount: String,
    base_rate: String,
    use_threshold: bool,
    rate_after_threshold: String,
    threshold_amount: String,
    opened_date: String,
    interest_accrual_date: String,
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
            "Даты открытия и начисления процентов сохраняются как справочные значения; процентный доход здесь не рассчитывается без истории остатка и правил начисления банка. Повышенная ставка показана как условие, а не как подтверждённая доходность.",
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
        self.form_snapshot = Some(self.form.clone());
        self.show_form = true;
        self.show_discard_confirmation = false;
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
        egui::ScrollArea::horizontal()
            .id_salt("savings_filters_scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Поиск");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Название или банк")
                            .desired_width(180.0),
                    );
                    if ui
                        .add_enabled(!self.search.is_empty(), egui::Button::new("×"))
                        .on_hover_text("Очистить поиск")
                        .clicked()
                    {
                        self.clear_search_filter();
                        should_refresh = true;
                    }
                    ui.add_space(8.0);

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
                        self.clear_bank_filter();
                        should_refresh = true;
                    }
                    ui.add_space(8.0);

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
                    if ui
                        .add_enabled(self.active_filter.is_some(), egui::Button::new("×"))
                        .on_hover_text("Очистить статус")
                        .clicked()
                    {
                        self.clear_active_filter();
                        should_refresh = true;
                    }
                    ui.add_space(8.0);

                    if ui.button("Применить").clicked() {
                        should_refresh = true;
                    }
                    let has_active_filters = !self.search.is_empty()
                        || self.bank_filter.is_some()
                        || self.active_filter.is_some();
                    if ui
                        .add_enabled(has_active_filters, egui::Button::new("Сбросить фильтры"))
                        .clicked()
                    {
                        self.reset_filters();
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
            let column_widths = savings_column_widths(
                (ui.available_width()
                    - ui.spacing().item_spacing.x * (SAVINGS_COLUMN_MIN_WIDTHS.len() - 1) as f32)
                    .max(0.0),
            );
            egui::ScrollArea::both()
                .id_salt("savings_table_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let row_height = TABLE_ROW_HEIGHT;
                    let table_width = column_widths.iter().sum::<f32>()
                        + ui.spacing().item_spacing.x * (column_widths.len() - 1) as f32;
                    ui.allocate_ui_with_layout(
                        egui::vec2(table_width, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(table_width);
                            for (heading, width) in [
                                "Банк",
                                "Название",
                                "Сумма",
                                "Базовая ставка",
                                "Ставка при условии",
                                "Порог",
                                "Срок",
                                "Дата открытия",
                                "Дата начисления процентов",
                                "Назначение процентов",
                                "Статус",
                                "Действия",
                            ]
                            .into_iter()
                            .zip(column_widths)
                            {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(width, row_height),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(width);
                                        ui.add(egui::Label::new(heading).wrap());
                                    },
                                );
                            }
                        },
                    );

                    for (row_index, account) in self
                        .entries
                        .iter()
                        .skip(self.current_page * SAVINGS_PAGE_SIZE)
                        .take(SAVINGS_PAGE_SIZE)
                        .enumerate()
                    {
                        let amount = account.amount.to_string();
                        let base_rate = format!("{}%", account.base_rate);
                        let rate_after_threshold = account
                            .rate_after_threshold
                            .map_or_else(|| "—".to_owned(), |rate| format!("{rate}%"));
                        let threshold_amount = account
                            .threshold_amount
                            .map_or_else(|| "—".to_owned(), |amount| format!("{amount}"));
                        let opened_date = account.opened_date.map_or_else(
                            || "—".to_owned(),
                            |date| date.format("%d.%m.%Y").to_string(),
                        );
                        let interest_accrual_date = account.interest_accrual_date.map_or_else(
                            || "—".to_owned(),
                            |date| date.format("%d.%m.%Y").to_string(),
                        );
                        let term = match (account.term_type, account.term_end_date) {
                            (SavingsTerm::Limited, Some(date)) => {
                                format!("до {}", date.format("%d.%m.%Y"))
                            }
                            (SavingsTerm::Limited, None) => "Дата не задана".to_owned(),
                            (SavingsTerm::Unlimited, _) => "Бессрочный".to_owned(),
                        };
                        let status = if account.is_active {
                            "Активно"
                        } else {
                            "Неактивно"
                        };
                        ui.allocate_ui_with_layout(
                            egui::vec2(table_width, row_height),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(table_width);
                                if row_index % 2 == 1 {
                                    ui.painter().rect_filled(
                                        ui.max_rect(),
                                        0.0,
                                        ui.visuals().faint_bg_color,
                                    );
                                }
                                for (value, width) in [
                                    account.bank_name.as_str(),
                                    account.title.as_str(),
                                    amount.as_str(),
                                    base_rate.as_str(),
                                    rate_after_threshold.as_str(),
                                    threshold_amount.as_str(),
                                    term.as_str(),
                                    opened_date.as_str(),
                                    interest_accrual_date.as_str(),
                                    account.interest_destination.title(),
                                    status,
                                ]
                                .into_iter()
                                .zip(column_widths)
                                {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(width, row_height),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.set_min_width(width);
                                            ui.add(egui::Label::new(value).wrap());
                                        },
                                    );
                                }
                                ui.allocate_ui_with_layout(
                                    egui::vec2(column_widths[11], row_height),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(column_widths[11]);
                                        if ui.small_button("Изменить").clicked() {
                                            action = Some(TableAction::Edit(account.id));
                                        }
                                        if ui.small_button("Удалить").clicked() {
                                            action = Some(TableAction::Delete(
                                                account.id,
                                                account.title.clone(),
                                            ));
                                        }
                                    },
                                );
                            },
                        );
                    }
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
            if let Some(result) = self.render_form(context, database, banks) {
                return Some(result);
            }
        }

        self.render_discard_confirmation(context);
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
            opened_date: account
                .opened_date
                .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string()),
            interest_accrual_date: account
                .interest_accrual_date
                .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string()),
            term_type: Some(account.term_type),
            term_end_date: account
                .term_end_date
                .map_or_else(String::new, |date| date.format("%Y-%m-%d").to_string()),
            interest_destination: Some(account.interest_destination),
            is_active: account.is_active,
            error: None,
        };
        self.form_snapshot = Some(self.form.clone());
        self.show_form = true;
        self.show_discard_confirmation = false;
    }

    fn clear_search_filter(&mut self) {
        self.search.clear();
    }

    fn clear_bank_filter(&mut self) {
        self.bank_filter = None;
    }

    fn clear_active_filter(&mut self) {
        self.active_filter = None;
    }

    fn reset_filters(&mut self) {
        self.clear_search_filter();
        self.clear_bank_filter();
        self.clear_active_filter();
    }

    fn render_form(
        &mut self,
        context: &egui::Context,
        database: &Database,
        banks: &[Bank],
    ) -> Option<(String, bool)> {
        let mut save_clicked = false;
        let mut cancel_clicked = false;
        let title = if self.form.id.is_some() {
            "Изменить накопление"
        } else {
            "Новое накопление"
        };
        let form_width = (context.screen_rect().width() - 64.0).clamp(480.0, 840.0);
        egui::Modal::new(egui::Id::new("savings_form_modal"))
            .frame(egui::Frame::popup(&context.style()).inner_margin(egui::Margin::same(16)))
            .show(context, |ui| {
                ui.set_min_width(form_width);
                ui.set_max_width(form_width);
                ui.spacing_mut().item_spacing = egui::vec2(16.0, 12.0);
                ui.heading(title);
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("savings_form_fields")
                    .max_height((context.screen_rect().height() - 240.0).max(180.0))
                    .show(ui, |ui| self.render_form_fields(ui, banks));

                if let Some(error) = &self.form.error {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::RED, error);
                }

                ui.add_space(8.0);
                ui.separator();
                let save_fill = ui.visuals().widgets.active.bg_fill;
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 248.0).max(0.0));
                    save_clicked = ui
                        .add_sized(
                            egui::vec2(120.0, 36.0),
                            egui::Button::new("Сохранить").fill(save_fill),
                        )
                        .clicked();
                    ui.add_space(8.0);
                    cancel_clicked = ui
                        .add_sized(egui::vec2(120.0, 36.0), egui::Button::new("Отмена"))
                        .clicked();
                });
            });
        if cancel_clicked {
            self.request_form_cancel();
            return None;
        }
        if !save_clicked {
            return None;
        }

        self.save_form(database)
    }

    fn render_form_fields(&mut self, ui: &mut egui::Ui, banks: &[Bank]) {
        ui.label(egui::RichText::new("Основное").strong());
        ui.columns(3, |columns| {
            let title_width = columns[0].available_width();
            render_savings_text_field(
                &mut columns[0],
                "Название",
                &mut self.form.title,
                title_width,
                None,
            );

            let bank_width = columns[1].available_width();
            columns[1].vertical(|ui| {
                ui.set_min_width(bank_width);
                ui.set_max_width(bank_width);
                ui.label("Банк");
                ui.add_space(4.0);
                let selected_bank = self
                    .form
                    .bank_id
                    .and_then(|id| banks.iter().find(|bank| bank.id == id))
                    .map_or("Выберите банк", |bank| bank.name.as_str());
                egui::ComboBox::from_id_salt("savings_form_bank")
                    .width(bank_width)
                    .selected_text(selected_bank)
                    .show_ui(ui, |ui| {
                        for bank in banks {
                            ui.selectable_value(&mut self.form.bank_id, Some(bank.id), &bank.name);
                        }
                    });
            });

            render_savings_checkbox_field(&mut columns[2], &mut self.form.is_active, "Активно");
        });

        ui.add_space(20.0);
        ui.label(egui::RichText::new("Сумма и ставки").strong());
        ui.columns(3, |columns| {
            let amount_width = columns[0].available_width();
            render_savings_text_field(
                &mut columns[0],
                "Сумма",
                &mut self.form.amount,
                amount_width,
                Some("Например, 100000"),
            );
            let base_rate_width = columns[1].available_width();
            render_savings_text_field(
                &mut columns[1],
                "Базовая ставка, %",
                &mut self.form.base_rate,
                base_rate_width,
                Some("Например, 5,5"),
            );
            render_savings_checkbox_field(
                &mut columns[2],
                &mut self.form.use_threshold,
                "Есть повышение ставки при выполнении условий",
            );
        });
        if self.form.use_threshold {
            ui.add_space(12.0);
            ui.columns(2, |columns| {
                let threshold_width = columns[0].available_width();
                render_savings_text_field(
                    &mut columns[0],
                    "Пороговая сумма трат",
                    &mut self.form.threshold_amount,
                    threshold_width,
                    None,
                );
                let threshold_rate_width = columns[1].available_width();
                render_savings_text_field(
                    &mut columns[1],
                    "Ставка при выполнении условий, %",
                    &mut self.form.rate_after_threshold,
                    threshold_rate_width,
                    None,
                );
            });
        }

        ui.add_space(20.0);
        ui.label(egui::RichText::new("Даты и срок").strong());
        ui.columns(3, |columns| {
            let opened_date_width = columns[0].available_width();
            render_savings_text_field(
                &mut columns[0],
                "Дата открытия (ГГГГ-ММ-ДД)",
                &mut self.form.opened_date,
                opened_date_width,
                Some("ГГГГ-ММ-ДД"),
            );
            let accrual_date_width = columns[1].available_width();
            render_savings_text_field(
                &mut columns[1],
                "Дата начисления процентов (ГГГГ-ММ-ДД)",
                &mut self.form.interest_accrual_date,
                accrual_date_width,
                Some("ГГГГ-ММ-ДД"),
            );
            let term_width = columns[2].available_width();
            columns[2].vertical(|ui| {
                ui.set_min_width(term_width);
                ui.set_max_width(term_width);
                ui.label("Срок");
                ui.add_space(4.0);
                egui::ComboBox::from_id_salt("savings_form_term")
                    .selected_text(
                        self.form
                            .term_type
                            .map_or("Выберите срок", SavingsTerm::title),
                    )
                    .width(term_width)
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
                    ui.add_space(8.0);
                    render_savings_text_field(
                        ui,
                        "Дата закрытия (ГГГГ-ММ-ДД)",
                        &mut self.form.term_end_date,
                        term_width,
                        Some("ГГГГ-ММ-ДД"),
                    );
                }
            });
        });

        ui.add_space(20.0);
        let destination_width = ui.available_width();
        ui.vertical(|ui| {
            ui.set_min_width(destination_width);
            ui.set_max_width(destination_width);
            ui.label("Выплата процентов");
            ui.add_space(4.0);
            egui::ComboBox::from_id_salt("savings_interest_destination")
                .width(destination_width)
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
    }

    fn save_form(&mut self, database: &Database) -> Option<(String, bool)> {
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
                self.form_snapshot = None;
                self.show_discard_confirmation = false;
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

    fn request_form_cancel(&mut self) {
        if self.form_has_unsaved_changes() {
            self.show_discard_confirmation = true;
        } else {
            self.close_form_without_saving();
        }
    }

    fn form_has_unsaved_changes(&self) -> bool {
        let Some(snapshot) = &self.form_snapshot else {
            return false;
        };
        let mut current = self.form.clone();
        let mut initial = snapshot.clone();
        current.error = None;
        initial.error = None;
        current != initial
    }

    fn close_form_without_saving(&mut self) {
        self.show_form = false;
        self.show_discard_confirmation = false;
        self.form_snapshot = None;
    }

    fn render_discard_confirmation(&mut self, context: &egui::Context) {
        if !self.show_discard_confirmation {
            return;
        }
        let mut keep_editing = false;
        let mut discard = false;
        egui::Modal::new(egui::Id::new("savings_discard_confirmation")).show(context, |ui| {
            ui.set_max_width(420.0);
            ui.heading("Несохранённые изменения");
            ui.label("Отменить изменения и закрыть форму?");
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 358.0).max(0.0));
                keep_editing = ui
                    .add_sized(
                        egui::vec2(190.0, 36.0),
                        egui::Button::new("Продолжить редактирование"),
                    )
                    .clicked();
                ui.add_space(8.0);
                discard = ui
                    .add_sized(
                        egui::vec2(160.0, 36.0),
                        egui::Button::new("Отменить изменения"),
                    )
                    .clicked();
            });
        });
        if keep_editing {
            self.resolve_form_cancel(false);
        } else if discard {
            self.resolve_form_cancel(true);
        }
    }

    fn resolve_form_cancel(&mut self, discard: bool) {
        self.show_discard_confirmation = false;
        if discard {
            self.close_form_without_saving();
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

fn render_savings_text_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    width: f32,
    hint: Option<&str>,
) {
    ui.vertical(|ui| {
        ui.set_min_width(width);
        ui.set_max_width(width);
        ui.label(label);
        ui.add_space(4.0);
        let mut field = egui::TextEdit::singleline(value).desired_width(width);
        if let Some(hint) = hint {
            field = field.hint_text(hint);
        }
        ui.add(field);
    });
}

fn render_savings_checkbox_field(ui: &mut egui::Ui, value: &mut bool, label: &str) {
    ui.vertical(|ui| {
        ui.add_space(ui.text_style_height(&egui::TextStyle::Body) + 4.0);
        ui.checkbox(value, label);
    });
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
        let opened_date = parse_optional_savings_date(&self.opened_date, "открытия")?;
        let interest_accrual_date =
            parse_optional_savings_date(&self.interest_accrual_date, "начисления процентов")?;
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
            opened_date,
            interest_accrual_date,
            term_type,
            term_end_date,
            interest_destination,
            is_active: self.is_active,
        };
        details.validate().map_err(|error| error.to_string())?;
        Ok(details)
    }
}

fn parse_optional_savings_date(value: &str, field_name: &str) -> Result<Option<NaiveDate>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map(Some)
        .map_err(|_| format!("Введите дату {field_name} в формате ГГГГ-ММ-ДД"))
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use eframe::egui;
    use finance_core::{Bank, InterestDestination, SavingsTerm};

    use super::{savings_column_widths, SavingsForm, SavingsPage, SAVINGS_COLUMN_MIN_WIDTHS};

    #[test]
    fn savings_column_widths_fill_available_space_or_keep_minimum_widths() {
        let minimum_width: f32 = SAVINGS_COLUMN_MIN_WIDTHS.iter().sum();

        for available_width in [1_280.0, 1_920.0, 700.0] {
            let widths = savings_column_widths(available_width);
            let table_width: f32 = widths.iter().sum();
            let expected_width = available_width.max(minimum_width);

            assert!((table_width - expected_width).abs() < 0.01);
            assert!(widths
                .iter()
                .zip(SAVINGS_COLUMN_MIN_WIDTHS)
                .all(|(width, minimum)| *width >= minimum));
        }
    }

    #[test]
    fn savings_table_renders_with_adaptive_columns_in_egui_context() {
        let (database, mut page) = filtered_savings_page();
        let context = egui::Context::default();
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
                        page.render(context, ui, &database, &[]);
                    });
                },
            ));
        }
        let output = output.expect("savings page should render twice");
        let rendered_text = output
            .shapes
            .iter()
            .filter_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) => Some(text_shape.galley.job.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let actions_header_x = output
            .shapes
            .iter()
            .find_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape)
                    if text_shape.galley.job.text == "Действия" =>
                {
                    Some(text_shape.pos.x)
                }
                _ => None,
            });
        assert!(rendered_text.contains(&"Ставка при условии"));
        assert!(rendered_text.contains(&"Дата открытия"));
        assert!(rendered_text.contains(&"Дата начисления процентов"));
        assert!(rendered_text.contains(&"Действия"));
        assert!(rendered_text.contains(&"Alpha"));
        assert!(rendered_text.contains(&"01.03.2026"));
        assert!(rendered_text.contains(&"Изменить"));
        assert!(
            actions_header_x.is_some_and(|x| x > 1_050.0 && x < 1_280.0),
            "actions header should be near the right edge: {actions_header_x:?}"
        );
    }

    #[test]
    fn savings_form_parses_optional_dates_and_allows_them_to_be_empty() {
        let mut form = valid_savings_form();
        form.opened_date = "2026-03-01".to_owned();
        form.interest_accrual_date = "2026-03-31".to_owned();

        let details = form.details().unwrap();

        assert_eq!(details.opened_date, NaiveDate::from_ymd_opt(2026, 3, 1));
        assert_eq!(
            details.interest_accrual_date,
            NaiveDate::from_ymd_opt(2026, 3, 31)
        );

        form.opened_date.clear();
        form.interest_accrual_date.clear();
        let details = form.details().unwrap();
        assert_eq!(details.opened_date, None);
        assert_eq!(details.interest_accrual_date, None);
    }

    #[test]
    fn savings_form_rejects_invalid_opening_and_accrual_dates() {
        let mut form = valid_savings_form();
        form.opened_date = "2026-02-30".to_owned();
        assert!(form.details().unwrap_err().contains("дату открытия"));

        form.opened_date.clear();
        form.interest_accrual_date = "not-a-date".to_owned();
        assert!(form
            .details()
            .unwrap_err()
            .contains("дату начисления процентов"));
    }

    #[test]
    fn editing_existing_savings_populates_all_saved_values() {
        let (_database, mut page) = filtered_savings_page();

        page.begin_edit(1);

        assert!(page.show_form);
        assert_eq!(page.form.id, Some(1));
        assert_eq!(page.form.bank_id, Some(1));
        assert_eq!(page.form.title, "Alpha");
        assert_eq!(page.form.amount, "100");
        assert_eq!(page.form.base_rate, "1");
        assert_eq!(page.form.opened_date, "2026-03-01");
        assert_eq!(page.form.interest_accrual_date, "2026-03-31");
        assert_eq!(page.form.term_type, Some(SavingsTerm::Unlimited));
        assert_eq!(
            page.form.interest_destination,
            Some(InterestDestination::Reinvest)
        );
        assert!(page.form.is_active);
    }

    #[test]
    fn editing_form_saves_changes_to_the_existing_savings_record() {
        let (database, mut page) = filtered_savings_page();
        page.begin_edit(1);
        page.form.title = "Изменённый вклад".to_owned();
        page.form.amount = "1250.50".to_owned();

        let result = page.save_form(&database);

        assert_eq!(result, Some(("Накопление сохранено".to_owned(), false)));
        assert!(!page.show_form);
        let entries = finance_db::list_savings(&database, None, None, "")
            .expect("saved savings should be readable without the previous filter");
        assert_eq!(entries.len(), 5);
        let updated = entries
            .iter()
            .find(|entry| entry.id == 1)
            .expect("edit should preserve the existing record id");
        assert_eq!(updated.title, "Изменённый вклад");
        assert_eq!(updated.amount.to_string(), "1250.5");
    }

    #[test]
    fn saving_validation_error_keeps_form_open_and_preserves_input() {
        let (database, mut page) = filtered_savings_page();
        page.begin_edit(1);
        page.form.amount = "не число".to_owned();

        let result = page.save_form(&database);

        assert_eq!(result, None);
        assert!(page.show_form);
        assert_eq!(page.form.amount, "не число");
        assert!(page.form.error.is_some());
    }

    #[test]
    fn saving_database_error_keeps_form_open_and_preserves_input() {
        let (database, mut page) = filtered_savings_page();
        page.begin_edit(1);
        page.form.bank_id = Some(999);
        page.form.title = "Несохранённое название".to_owned();

        let result = page.save_form(&database);

        assert_eq!(result, None);
        assert!(page.show_form);
        assert_eq!(page.form.bank_id, Some(999));
        assert_eq!(page.form.title, "Несохранённое название");
        assert!(page.form.error.is_some());
        assert_eq!(page.entries[0].title, "Alpha");
    }

    #[test]
    fn cancelling_edited_form_requires_confirmation_before_discarding() {
        let (_database, mut page) = filtered_savings_page();
        page.begin_edit(1);
        page.request_form_cancel();
        assert!(!page.show_form);

        page.begin_edit(1);
        page.form.title = "Черновик".to_owned();
        page.request_form_cancel();

        assert!(page.show_form);
        assert!(page.show_discard_confirmation);
        page.resolve_form_cancel(false);
        assert!(page.show_form);
        assert!(!page.show_discard_confirmation);

        page.request_form_cancel();
        page.resolve_form_cancel(true);
        assert!(!page.show_form);
        assert!(!page.show_discard_confirmation);
        assert!(page.form_snapshot.is_none());
    }

    #[test]
    fn savings_edit_form_renders_with_available_actions_at_minimum_window_size() {
        let (database, mut page) = filtered_savings_page();
        page.begin_edit(1);
        page.form.use_threshold = true;
        page.form.term_type = Some(SavingsTerm::Limited);
        page.form.term_end_date = "2027-03-01".to_owned();
        let banks = [Bank {
            id: 1,
            name: "Банк Альфа".to_owned(),
            comment: String::new(),
        }];
        let context = egui::Context::default();
        let output = render_savings_fields(&context, &mut page, &banks);
        assert!(page.show_form);
        let text_shapes = rendered_text_positions(&output);

        let title = rendered_position(&text_shapes, "Название");
        let bank = rendered_position(&text_shapes, "Банк");
        let active = rendered_position(&text_shapes, "Активно");
        assert!(title.x < bank.x && bank.x < active.x);
        assert!((title.y - bank.y).abs() < 1.0);

        let amount = rendered_position(&text_shapes, "Сумма");
        let base_rate = rendered_position(&text_shapes, "Базовая ставка, %");
        let threshold_enabled =
            rendered_position(&text_shapes, "Есть повышение ставки при выполнении условий");
        assert!(amount.x < base_rate.x && base_rate.x < threshold_enabled.x);
        assert!((amount.y - base_rate.y).abs() < 1.0);
        assert!(base_rate.x - amount.x > 150.0);
        assert!(threshold_enabled.x - base_rate.x > 150.0);

        let threshold_amount = rendered_position(&text_shapes, "Пороговая сумма трат");
        let threshold_rate = rendered_position(&text_shapes, "Ставка при выполнении условий, %");
        assert!(threshold_amount.x < threshold_rate.x);
        assert!((threshold_amount.y - threshold_rate.y).abs() < 1.0);

        let opened_date = rendered_position(&text_shapes, "Дата открытия (ГГГГ-ММ-ДД)");
        let accrual_date =
            rendered_position(&text_shapes, "Дата начисления процентов (ГГГГ-ММ-ДД)");
        let term = rendered_position(&text_shapes, "Срок");
        let close_date = rendered_position(&text_shapes, "Дата закрытия (ГГГГ-ММ-ДД)");
        let payout = rendered_position(&text_shapes, "Выплата процентов");
        assert!(opened_date.x < accrual_date.x && accrual_date.x < term.x);
        assert!((opened_date.y - accrual_date.y).abs() < 1.0);
        assert!(threshold_rate.y < opened_date.y);
        assert!(term.y < close_date.y && close_date.y < payout.y);

        let modal_context = egui::Context::default();
        render_form_at_size(
            &modal_context,
            &mut page,
            &database,
            &banks,
            egui::vec2(1_280.0, 720.0),
        );
        let minimum_output = render_form_at_size(
            &modal_context,
            &mut page,
            &database,
            &banks,
            egui::vec2(1_280.0, 720.0),
        );
        let minimum_text_shapes = rendered_text_positions(&minimum_output);
        let save = rendered_position(&minimum_text_shapes, "Сохранить");
        let cancel = rendered_position(&minimum_text_shapes, "Отмена");
        assert!(save.x < cancel.x);
        assert!((save.y - cancel.y).abs() < 1.0);
        assert!(save.x >= 0.0 && cancel.x < 1_280.0);
        assert!(save.y >= 0.0 && cancel.y < 720.0);

        page.form.use_threshold = false;
        page.form.term_type = Some(SavingsTerm::Unlimited);
        let output = render_savings_fields(&context, &mut page, &banks);
        let text_shapes = rendered_text_positions(&output);
        assert!(!text_shapes.iter().any(|(text, _)| {
            text == "Пороговая сумма трат" || text == "Ставка при выполнении условий, %"
        }));
        assert!(!text_shapes
            .iter()
            .any(|(text, _)| text == "Дата закрытия (ГГГГ-ММ-ДД)"));
        assert!(text_shapes
            .iter()
            .any(|(text, _)| text == "Выплата процентов"));
    }

    fn render_form_at_size(
        context: &egui::Context,
        page: &mut SavingsPage,
        database: &finance_db::Database,
        banks: &[Bank],
        screen_size: egui::Vec2,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_size)),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    page.render(context, ui, database, banks);
                });
            },
        )
    }

    fn render_savings_fields(
        context: &egui::Context,
        page: &mut SavingsPage,
        banks: &[Bank],
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(840.0, 1_200.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    page.render_form_fields(ui, banks);
                });
            },
        )
    }

    fn rendered_text_positions(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
        output
            .shapes
            .iter()
            .filter_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) => {
                    Some((text_shape.galley.job.text.clone(), text_shape.pos))
                }
                _ => None,
            })
            .collect()
    }

    fn rendered_position(shapes: &[(String, egui::Pos2)], text: &str) -> egui::Pos2 {
        shapes
            .iter()
            .rev()
            .find_map(|(rendered_text, position)| (rendered_text == text).then_some(*position))
            .unwrap_or_else(|| {
                panic!(
                    "expected text {text:?} to be rendered, got: {:?}",
                    shapes.iter().map(|(text, _)| text).collect::<Vec<_>>()
                )
            })
    }

    fn valid_savings_form() -> SavingsForm {
        SavingsForm {
            bank_id: Some(1),
            title: "Тестовый вклад".to_owned(),
            amount: "1000".to_owned(),
            base_rate: "5".to_owned(),
            term_type: Some(SavingsTerm::Unlimited),
            interest_destination: Some(InterestDestination::Reinvest),
            is_active: true,
            ..SavingsForm::default()
        }
    }

    fn filtered_savings_page() -> (finance_db::Database, SavingsPage) {
        let database =
            finance_db::Database::open_in_memory().expect("in-memory SQLite should open");
        database
            .execute_batch(include_str!("../../db/migrations/V1__initial.sql"))
            .expect("initial migration should apply");
        database
            .execute_batch(include_str!(
                "../../db/migrations/V2__transactions_and_directories.sql"
            ))
            .expect("directory migration should apply");
        database
            .execute_batch(include_str!("../../db/migrations/V3__savings.sql"))
            .expect("savings migration should apply");
        database
            .execute_batch(include_str!(
                "../../db/migrations/V4__payment_documents.sql"
            ))
            .expect("payment document migration should apply");
        database
            .execute_batch(include_str!("../../db/migrations/V5__savings_dates.sql"))
            .expect("savings date migration should apply");
        database
            .execute_batch(
                "INSERT INTO banks (id, name) VALUES (1, 'Банк Альфа'), (2, 'Банк Бета');
                 INSERT INTO savings (
                     id, bank_id, title, amount, base_rate, term_type,
                     term_end_date, opened_date, interest_accrual_date,
                     interest_destination, is_active
                 ) VALUES
                     (1, 1, 'Alpha', '100', '1', 'unlimited', NULL, '2026-03-01', '2026-03-31', 'reinvest', 1),
                     (2, 1, 'Alpha', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 0),
                     (3, 1, 'Gamma', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 1),
                     (4, 2, 'Alpha', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 1),
                     (5, 2, 'Beta', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 0);",
            )
            .expect("savings fixtures should be inserted");

        let mut page = SavingsPage {
            bank_filter: Some(1),
            active_filter: Some(true),
            search: "Alpha".to_owned(),
            ..SavingsPage::default()
        };
        page.refresh(&database)
            .expect("initial savings filter should load");
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [1]
        );

        (database, page)
    }

    #[test]
    fn clearing_each_savings_filter_refreshes_results_and_preserves_other_filters() {
        let (database, mut page) = filtered_savings_page();
        page.clear_search_filter();
        page.refresh(&database)
            .expect("results without search should load");
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [1, 3]
        );

        let (database, mut page) = filtered_savings_page();
        page.clear_bank_filter();
        page.refresh(&database)
            .expect("results without bank filter should load");
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [1, 4]
        );

        let (database, mut page) = filtered_savings_page();
        page.clear_active_filter();
        page.refresh(&database)
            .expect("results without status filter should load");
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn resetting_savings_filters_clears_all_values_and_refreshes_results() {
        let (database, mut page) = filtered_savings_page();
        page.reset_filters();
        page.refresh(&database)
            .expect("results without filters should load");

        assert!(page.search.is_empty());
        assert_eq!(page.bank_filter, None);
        assert_eq!(page.active_filter, None);
        assert_eq!(page.entries.len(), 5);
    }
}
