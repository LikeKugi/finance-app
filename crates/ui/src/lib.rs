use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    parse_amount_expression, Bank, Category, Currency, FinancialTransaction, TransactionKind,
};
use finance_db::{self, Database};
use rust_decimal::Decimal;

mod date_input;

use date_input::{date_input, DateInputState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModuleTab {
    Income,
    Expenses,
    Savings,
    PaymentDocuments,
    Analysis,
    Directories,
}

impl ModuleTab {
    const ALL: [Self; 6] = [
        Self::Income,
        Self::Expenses,
        Self::Savings,
        Self::PaymentDocuments,
        Self::Analysis,
        Self::Directories,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Income => "Доходы",
            Self::Expenses => "Расходы",
            Self::Savings => "Накопления",
            Self::PaymentDocuments => "Платёжные документы",
            Self::Analysis => "Анализ",
            Self::Directories => "Справочники",
        }
    }

    const fn transaction_kind(self) -> Option<TransactionKind> {
        match self {
            Self::Income => Some(TransactionKind::Income),
            Self::Expenses => Some(TransactionKind::Expense),
            Self::Savings | Self::PaymentDocuments | Self::Analysis | Self::Directories => None,
        }
    }
}

#[derive(Clone, Copy)]
enum DeleteTarget {
    Transaction(TransactionKind, i64),
    Category(TransactionKind, i64),
    Bank(i64),
}

#[derive(Default)]
struct TransactionForm {
    id: Option<i64>,
    date: String,
    category_id: Option<i64>,
    amount: String,
    currency: Option<Currency>,
    comment: String,
}

#[derive(Default)]
struct CategoryForm {
    id: Option<i64>,
    name: String,
    parent_id: Option<i64>,
    is_active: bool,
}

#[derive(Default)]
struct BankForm {
    id: Option<i64>,
    name: String,
    comment: String,
}

struct FinanceApp {
    database: Database,
    active_tab: ModuleTab,
    transactions: Vec<FinancialTransaction>,
    categories: Vec<Category>,
    banks: Vec<Bank>,
    transaction_form: TransactionForm,
    category_form: CategoryForm,
    bank_form: BankForm,
    directory_kind: TransactionKind,
    search: String,
    from_date: String,
    to_date: String,
    transaction_date_input: DateInputState,
    from_date_input: DateInputState,
    to_date_input: DateInputState,
    category_filter: Option<i64>,
    show_transaction_form: bool,
    show_category_form: bool,
    show_bank_form: bool,
    pending_delete: Option<DeleteTarget>,
    message: Option<(String, bool)>,
}

const TRANSACTION_FILTERS_SCROLL_AREA_ID: &str = "transaction_filters_scroll_area";
const TRANSACTION_TABLE_SCROLL_AREA_ID: &str = "transaction_table_scroll_area";
const DIRECTORY_CATEGORIES_SCROLL_AREA_ID: &str = "directory_categories_scroll_area";
const DIRECTORY_BANKS_SCROLL_AREA_ID: &str = "directory_banks_scroll_area";

impl FinanceApp {
    fn new(database: Database) -> Self {
        let mut app = Self {
            database,
            active_tab: ModuleTab::Income,
            transactions: Vec::new(),
            categories: Vec::new(),
            banks: Vec::new(),
            transaction_form: TransactionForm::default(),
            category_form: CategoryForm::default(),
            bank_form: BankForm::default(),
            directory_kind: TransactionKind::Income,
            search: String::new(),
            from_date: String::new(),
            to_date: String::new(),
            transaction_date_input: DateInputState::default(),
            from_date_input: DateInputState::default(),
            to_date_input: DateInputState::default(),
            category_filter: None,
            show_transaction_form: false,
            show_category_form: false,
            show_bank_form: false,
            pending_delete: None,
            message: None,
        };
        app.refresh_all();
        app
    }

    fn refresh_all(&mut self) {
        if let Some(kind) = self.active_tab.transaction_kind() {
            self.refresh_transactions(kind);
            self.refresh_categories(kind, true);
        }
        if self.active_tab == ModuleTab::Directories {
            self.refresh_directories();
        }
    }

    fn refresh_transactions(&mut self, kind: TransactionKind) {
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
            self.message = Some(("Начало периода не может быть позже окончания".to_owned(), true));
            return;
        }
        match finance_db::list_transactions(
            &self.database,
            kind,
            from,
            to,
            self.category_filter,
            &self.search,
        ) {
            Ok(transactions) => {
                self.transactions = transactions;
                self.message = None;
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn refresh_categories(&mut self, kind: TransactionKind, include_inactive: bool) {
        match finance_db::list_categories(&self.database, kind, include_inactive) {
            Ok(categories) => self.categories = categories,
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn refresh_directories(&mut self) {
        self.refresh_categories(self.directory_kind, true);
        match finance_db::list_banks(&self.database) {
            Ok(banks) => self.banks = banks,
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn show_error(&mut self, message: String) {
        self.message = Some((message, true));
    }

    fn show_success(&mut self, message: &str) {
        self.message = Some((message.to_owned(), false));
    }

    fn start_new_transaction(&mut self) {
        self.transaction_date_input.reset();
        self.transaction_form = TransactionForm {
            date: chrono::Local::now().date_naive().format("%Y-%m-%d").to_string(),
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

    fn edit_transaction(&mut self, transaction: &FinancialTransaction) {
        self.transaction_date_input.reset();
        self.transaction_form = TransactionForm {
            id: Some(transaction.id),
            date: transaction.date.format("%Y-%m-%d").to_string(),
            category_id: Some(transaction.category_id),
            amount: transaction.amount.to_string(),
            currency: Some(transaction.currency),
            comment: transaction.comment.clone(),
        };
        self.show_transaction_form = true;
    }

    fn save_transaction(&mut self, kind: TransactionKind) {
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
        match finance_db::save_transaction(
            &self.database,
            kind,
            self.transaction_form.id,
            date,
            category_id,
            amount,
            currency,
            &self.transaction_form.comment,
        ) {
            Ok(_) => {
                self.show_transaction_form = false;
                self.show_success("Операция сохранена");
                self.refresh_transactions(kind);
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn save_category(&mut self) {
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

    fn save_bank(&mut self) {
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

    fn confirm_delete(&mut self) {
        let Some(target) = self.pending_delete.take() else {
            return;
        };
        let result = match target {
            DeleteTarget::Transaction(kind, id) => {
                finance_db::delete_transaction(&self.database, kind, id)
            }
            DeleteTarget::Category(kind, id) => finance_db::delete_category(&self.database, kind, id),
            DeleteTarget::Bank(id) => finance_db::delete_bank(&self.database, id),
        };
        match result {
            Ok(()) => {
                self.show_success("Запись удалена");
                self.refresh_all();
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn render_transactions(&mut self, ui: &mut egui::Ui, kind: TransactionKind) {
        egui::ScrollArea::horizontal()
            .id_salt(TRANSACTION_FILTERS_SCROLL_AREA_ID)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Поиск");
                    ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(150.0));
                    ui.add_space(8.0);
                    ui.label("С");
                    date_input(
                        ui,
                        "from_date_filter",
                        &mut self.from_date,
                        &mut self.from_date_input,
                        95.0,
                    );
                    ui.add_space(8.0);
                    ui.label("По");
                    date_input(
                        ui,
                        "to_date_filter",
                        &mut self.to_date,
                        &mut self.to_date_input,
                        95.0,
                    );
                    ui.add_space(8.0);
                    ui.label("Категория:");
                    let selected_name = self.category_filter
                        .and_then(|id| self.categories.iter().find(|category| category.id == id))
                        .map_or("Все", |category| category.name.as_str());
                    egui::ComboBox::from_id_salt("transaction_category_filter")
                        .width(150.0)
                        .selected_text(selected_name)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut self.category_filter, None, "Все")
                                .clicked()
                            {
                                self.refresh_transactions(kind);
                            }
                            let categories = self.categories.clone();
                            for category in categories {
                                if ui
                                    .selectable_value(
                                        &mut self.category_filter,
                                        Some(category.id),
                                        &category.name,
                                    )
                                    .clicked()
                                {
                                    self.refresh_transactions(kind);
                                }
                            }
                        });
                    ui.add_space(8.0);
                    if ui.button("Применить").clicked() {
                        self.refresh_transactions(kind);
                    }
                    if !self.categories.iter().any(|category| category.is_active) {
                        ui.add_space(8.0);
                        ui.label("Сначала добавьте категорию в разделе «Справочники».");
                    }
                });
            });

        if self.show_transaction_form {
            self.render_transaction_form(ui, kind);
        }

        ui.separator();

        let column_spacing = ui.spacing().item_spacing.x;
        let comment_width = (ui.available_width()
            - column_spacing * 5.0
            - 85.0
            - 180.0
            - 130.0
            - 75.0
            - 150.0)
            .max(120.0);
        egui::ScrollArea::vertical()
            .id_salt(TRANSACTION_TABLE_SCROLL_AREA_ID)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("transactions_table")
                    .striped(true)
                    .min_col_width(0.0)
                    .show(ui, |ui| {
                        for heading in [
                            "Дата",
                            "Сумма",
                            "Валюта",
                            "Категория",
                            "Комментарий",
                            "Действия",
                        ] {
                            ui.strong(heading);
                        }
                        ui.end_row();
                        let transactions = self.transactions.clone();
                        for transaction in &transactions {
                            ui.label(transaction.date.format("%d.%m.%Y").to_string());
                            ui.add_sized(
                                [130.0, 0.0],
                                egui::Label::new(transaction.amount.to_string()).wrap(),
                            );
                            ui.label(transaction.currency.code());
                            ui.add_sized(
                                [180.0, 0.0],
                                egui::Label::new(&transaction.category_name).wrap(),
                            );
                            ui.add_sized(
                                [comment_width, 0.0],
                                egui::Label::new(&transaction.comment).wrap(),
                            );
                            ui.horizontal(|ui| {
                                if ui.small_button("Изменить").clicked() {
                                    self.edit_transaction(transaction);
                                }
                                if ui.small_button("Удалить").clicked() {
                                    self.pending_delete =
                                        Some(DeleteTarget::Transaction(kind, transaction.id));
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
    }

    fn render_transaction_form(&mut self, ui: &mut egui::Ui, kind: TransactionKind) {
        ui.separator();
        ui.heading(if self.transaction_form.id.is_some() { "Изменить операцию" } else { "Новая операция" });
        ui.horizontal(|ui| {
            ui.label("Дата");
            date_input(
                ui,
                "transaction_date",
                &mut self.transaction_form.date,
                &mut self.transaction_date_input,
                95.0,
            );
            ui.label("Категория");
            let selected_category = self.transaction_form.category_id
                .and_then(|id| self.categories.iter().find(|category| category.id == id))
                .map_or("Выберите", |category| category.name.as_str());
            egui::ComboBox::from_id_salt("transaction_category_form")
                .selected_text(selected_category)
                .show_ui(ui, |ui| {
                    for category in &self.categories {
                        if category.is_active || Some(category.id) == self.transaction_form.category_id {
                            ui.selectable_value(&mut self.transaction_form.category_id, Some(category.id), &category.name);
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Сумма");
            ui.add(
                egui::TextEdit::singleline(&mut self.transaction_form.amount)
                    .desired_width(180.0)
                    .hint_text("например: 100 + 200 + 347"),
            );
            ui.label("Валюта");
            let selected_currency = self.transaction_form.currency.map_or("Выберите", Currency::code);
            egui::ComboBox::from_id_salt("transaction_currency")
                .selected_text(selected_currency)
                .show_ui(ui, |ui| {
                    for currency in Currency::ALL {
                        ui.selectable_value(&mut self.transaction_form.currency, Some(currency), currency.code());
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Комментарий");
            ui.add(
                egui::TextEdit::multiline(&mut self.transaction_form.comment)
                    .desired_width(480.0)
                    .desired_rows(3),
            );
        });
        if !self.transaction_form.amount.trim().is_empty() {
            match parse_amount_expression(&self.transaction_form.amount) {
                Ok(total) => {
                    ui.label(format!("Итого: {total}"));
                }
                Err(_) => {
                    ui.colored_label(
                        egui::Color32::RED,
                        "Введите положительные суммы, разделённые знаком +",
                    );
                }
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Сохранить").clicked() {
                self.save_transaction(kind);
            }
            if ui.button("Отмена").clicked() {
                self.show_transaction_form = false;
            }
        });
    }

    fn render_directories(&mut self, ui: &mut egui::Ui) -> (egui::Id, egui::Id) {
        let previous_kind = self.directory_kind;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.directory_kind, TransactionKind::Income, "Категории доходов");
            ui.selectable_value(&mut self.directory_kind, TransactionKind::Expense, "Категории расходов");
        });
        if self.directory_kind != previous_kind {
            self.category_form = CategoryForm::default();
            self.show_category_form = false;
            self.refresh_categories(self.directory_kind, true);
        }
        let mut scroll_area_ids = None;
        ui.columns(2, |columns| {
            columns[0].horizontal(|ui| {
                ui.heading("Категории");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("＋ Добавить категорию").clicked() {
                        self.category_form = CategoryForm { is_active: true, ..CategoryForm::default() };
                        self.show_category_form = true;
                    }
                });
            });
            let category_scroll_area =
                egui::ScrollArea::vertical().id_salt(DIRECTORY_CATEGORIES_SCROLL_AREA_ID);
            let category_scroll_output = category_scroll_area.show(&mut columns[0], |ui| {
                let categories = self.categories.clone();
                let category_rows = category_tree_rows(&categories);
                let name_width = (ui.available_width() - 145.0).max(100.0);
                egui::Grid::new("directory_categories_table")
                    .striped(true)
                    .min_col_width(85.0)
                    .show(ui, |ui| {
                        ui.add_sized(
                            [name_width, 20.0],
                            egui::Label::new(egui::RichText::new("Категория").strong()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.strong("Действия");
                        });
                        ui.end_row();

                        for (index, depth) in category_rows {
                            let category = &categories[index];
                            let prefix = if depth == 0 {
                                String::new()
                            } else {
                                format!("{}↳ ", "　".repeat(depth))
                            };
                            let active_suffix = if category.is_active { "" } else { " (неактивна)" };
                            ui.add_sized(
                                [name_width, 20.0],
                                egui::Label::new(format!("{prefix}{}{active_suffix}", category.name)),
                            );

                            let mut edit_clicked = false;
                            let mut delete_clicked = false;
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                delete_clicked = ui.small_button("Удалить").clicked();
                                edit_clicked = ui.small_button("Изменить").clicked();
                            });
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
                                self.pending_delete = Some(DeleteTarget::Category(self.directory_kind, category.id));
                            }
                            ui.end_row();
                        }
                    });
            });
            columns[1].horizontal(|ui| {
                ui.heading("Банки");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("＋ Добавить банк").clicked() {
                        self.bank_form = BankForm::default();
                        self.show_bank_form = true;
                    }
                });
            });
            let bank_scroll_area = egui::ScrollArea::vertical().id_salt(DIRECTORY_BANKS_SCROLL_AREA_ID);
            let bank_scroll_output = bank_scroll_area.show(&mut columns[1], |ui| {
                let banks = self.banks.clone();
                let name_width = (ui.available_width() - 145.0).max(100.0);
                egui::Grid::new("directory_banks_table")
                    .striped(true)
                    .min_col_width(85.0)
                    .show(ui, |ui| {
                        ui.add_sized(
                            [name_width, 20.0],
                            egui::Label::new(egui::RichText::new("Банк").strong()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.strong("Действия");
                        });
                        ui.end_row();

                        for bank in &banks {
                            ui.add_sized([name_width, 20.0], egui::Label::new(&bank.name));

                            let mut edit_clicked = false;
                            let mut delete_clicked = false;
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                delete_clicked = ui.small_button("Удалить").clicked();
                                edit_clicked = ui.small_button("Изменить").clicked();
                            });
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
                            ui.end_row();
                        }
                    });
            });
            scroll_area_ids = Some((category_scroll_output.id, bank_scroll_output.id));
        });

        if self.show_category_form {
            self.render_category_form(ui);
        }
        if self.show_bank_form {
            self.render_bank_form(ui);
        }
        scroll_area_ids.expect("directory columns should render both scroll areas")
    }

    fn render_category_form(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading(if self.category_form.id.is_some() { "Изменить категорию" } else { "Новая категория" });
        ui.horizontal(|ui| {
            ui.label("Название");
            ui.text_edit_singleline(&mut self.category_form.name);
            ui.label("Родительская категория");
            let selected_parent = self.category_form.parent_id
                .and_then(|id| self.categories.iter().find(|category| category.id == id))
                .map_or("Нет", |category| category.name.as_str());
            egui::ComboBox::from_id_salt("category_parent")
                .selected_text(selected_parent)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category_form.parent_id, None, "Нет");
                    for category in &self.categories {
                        if Some(category.id) != self.category_form.id {
                            ui.selectable_value(&mut self.category_form.parent_id, Some(category.id), &category.name);
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

    fn render_bank_form(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading(if self.bank_form.id.is_some() { "Изменить банк" } else { "Новый банк" });
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

    fn render_delete_confirmation(&mut self, context: &egui::Context) {
        let Some(target) = self.pending_delete else {
            return;
        };
        egui::Window::new("Подтверждение удаления")
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label("Удалить выбранную запись? Это действие нельзя отменить.");
                ui.horizontal(|ui| {
                    if ui.button("Удалить").clicked() {
                        self.pending_delete = Some(target);
                        self.confirm_delete();
                    }
                    if ui.button("Отмена").clicked() {
                        self.pending_delete = None;
                    }
                });
            });
    }
}

impl eframe::App for FinanceApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("module_tabs").show(context, |ui| {
            ui.horizontal_wrapped(|ui| {
                for tab in ModuleTab::ALL {
                    if ui.selectable_value(&mut self.active_tab, tab, tab.title()).clicked() {
                        self.transaction_form = TransactionForm::default();
                        self.show_transaction_form = false;
                        self.category_filter = None;
                        self.refresh_all();
                    }
                }
            });
        });

        if self.active_tab.transaction_kind().is_some() {
            egui::TopBottomPanel::bottom("transaction_totals").show(context, |ui| {
                ui.separator();
                match self.transactions.iter().try_fold(
                    (Decimal::ZERO, Decimal::ZERO),
                    |mut totals, transaction| {
                        let total = match transaction.currency {
                            Currency::Rub => &mut totals.0,
                            Currency::Usd => &mut totals.1,
                        };
                        *total = total.checked_add(transaction.amount)?;
                        Some(totals)
                    },
                ) {
                    Some((rub, usd)) => ui.label(format!(
                        "Записей: {}   Итого: {} RUB   {} USD",
                        self.transactions.len(),
                        rub,
                        usd
                    )),
                    None => {
                        self.show_error("Переполнение при подсчёте итоговой суммы".to_owned());
                        ui.colored_label(
                            egui::Color32::RED,
                            "Не удалось рассчитать итог: сумма слишком велика",
                        )
                    }
                };
            });
        }

        egui::CentralPanel::default().show(context, |ui| {
            ui.horizontal(|ui| {
                ui.heading(self.active_tab.title());
                let transaction_kind = match self.active_tab {
                    ModuleTab::Income => Some(TransactionKind::Income),
                    ModuleTab::Expenses => Some(TransactionKind::Expense),
                    _ => None,
                };
                if let Some(kind) = transaction_kind {
                    let actions_width = 140.0 * 2.0 + ui.spacing().item_spacing.x;
                    ui.add_space((ui.available_width() - actions_width).max(0.0));
                    if ui
                        .add_sized(egui::vec2(140.0, 40.0), egui::Button::new("＋ Добавить"))
                        .clicked()
                    {
                        self.refresh_categories(kind, true);
                        self.start_new_transaction();
                    }
                    if ui
                        .add_sized(egui::vec2(140.0, 40.0), egui::Button::new("↻ Обновить"))
                        .clicked()
                    {
                        self.refresh_transactions(kind);
                    }
                } else if self.active_tab == ModuleTab::Directories {
                    let actions_width = 140.0;
                    ui.add_space((ui.available_width() - actions_width).max(0.0));
                    if ui
                        .add_sized(egui::vec2(140.0, 40.0), egui::Button::new("↻ Обновить"))
                        .clicked()
                    {
                        self.refresh_directories();
                    }
                }
            });
            ui.separator();
            if let Some((message, is_error)) = &self.message {
                ui.colored_label(
                    if *is_error { egui::Color32::RED } else { egui::Color32::DARK_GREEN },
                    message,
                );
                ui.separator();
            }
            match self.active_tab {
                ModuleTab::Income => self.render_transactions(ui, TransactionKind::Income),
                ModuleTab::Expenses => self.render_transactions(ui, TransactionKind::Expense),
                ModuleTab::Directories => {
                    self.render_directories(ui);
                }
                ModuleTab::Savings | ModuleTab::PaymentDocuments | ModuleTab::Analysis => {
                    ui.label("Этот раздел пока не реализован.");
                }
            }
        });
        self.render_delete_confirmation(context);
    }
}

fn category_tree_rows(categories: &[Category]) -> Vec<(usize, usize)> {
    let mut category_ids = HashSet::with_capacity(categories.len());
    let mut children_by_parent = HashMap::<i64, Vec<usize>>::new();
    let mut roots = Vec::new();

    for category in categories {
        category_ids.insert(category.id);
    }
    for (index, category) in categories.iter().enumerate() {
        match category.parent_id {
            Some(parent_id) if category_ids.contains(&parent_id) => {
                children_by_parent.entry(parent_id).or_default().push(index);
            }
            _ => roots.push(index),
        }
    }

    let mut rows = Vec::with_capacity(categories.len());
    let mut visited = vec![false; categories.len()];
    for root in roots.into_iter().chain(0..categories.len()) {
        if visited[root] {
            continue;
        }

        let mut pending = vec![(root, 0)];
        while let Some((index, depth)) = pending.pop() {
            if visited[index] {
                continue;
            }

            visited[index] = true;
            rows.push((index, depth));
            if let Some(children) = children_by_parent.get(&categories[index].id) {
                pending.extend(children.iter().rev().map(|&child| (child, depth + 1)));
            }
        }
    }

    rows
}

fn parse_optional_date(value: &str) -> Result<Option<NaiveDate>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map(Some)
        .map_err(|_| "Введите дату фильтра в формате ГГГГ-ММ-ДД".to_owned())
}

/// Starts the native desktop window with the initialized local database.
pub fn run(database: Database) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([1280.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "FinanceApp — управление финансами",
        options,
        Box::new(move |_creation_context| Ok(Box::new(FinanceApp::new(database)))),
    )
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use super::{
        category_tree_rows, Category, FinanceApp, ModuleTab, DIRECTORY_BANKS_SCROLL_AREA_ID,
        DIRECTORY_CATEGORIES_SCROLL_AREA_ID, TRANSACTION_FILTERS_SCROLL_AREA_ID,
        TRANSACTION_TABLE_SCROLL_AREA_ID,
    };

    fn category(id: i64, parent_id: Option<i64>, is_active: bool) -> Category {
        Category {
            id,
            name: format!("Категория {id}"),
            parent_id,
            is_active,
        }
    }

    #[test]
    fn category_tree_rows_nest_children_and_keep_sibling_order() {
        let categories = vec![
            category(3, Some(2), true),
            category(20, None, true),
            category(4, Some(3), true),
            category(2, None, true),
            category(5, Some(2), true),
        ];

        let rows: Vec<_> = category_tree_rows(&categories)
            .into_iter()
            .map(|(index, depth)| (categories[index].id, depth))
            .collect();

        assert_eq!(rows, vec![(20, 0), (2, 0), (3, 1), (4, 2), (5, 1)]);
    }

    #[test]
    fn category_tree_rows_keep_inactive_categories_visible() {
        let categories = vec![category(1, None, true), category(2, Some(1), false)];

        let rows: Vec<_> = category_tree_rows(&categories)
            .into_iter()
            .map(|(index, depth)| (categories[index].id, depth, categories[index].is_active))
            .collect();

        assert_eq!(rows, vec![(1, 0, true), (2, 1, false)]);
    }

    #[test]
    fn category_tree_rows_include_missing_parents_and_cycles() {
        let categories = vec![
            category(1, Some(2), true),
            category(2, Some(1), true),
            category(3, Some(99), true),
            category(4, Some(3), true),
            category(5, None, true),
        ];

        let rows: Vec<_> = category_tree_rows(&categories)
            .into_iter()
            .map(|(index, depth)| (categories[index].id, depth))
            .collect();

        assert_eq!(rows, vec![(3, 0), (4, 1), (5, 0), (1, 0), (2, 1)]);
    }

    #[test]
    fn transaction_scroll_areas_have_distinct_ids() {
        let context = egui::Context::default();
        let mut scroll_area_ids = None;

        let _output = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let filters = egui::ScrollArea::horizontal()
                    .id_salt(TRANSACTION_FILTERS_SCROLL_AREA_ID)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label("Фильтры");
                    });
                let table = egui::ScrollArea::vertical()
                    .id_salt(TRANSACTION_TABLE_SCROLL_AREA_ID)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label("Таблица");
                    });
                scroll_area_ids = Some((filters.id, table.id));
            });
        });

        let (filters_id, table_id) = scroll_area_ids.expect("egui should render both scroll areas");
        assert_ne!(filters_id, table_id);
    }

    #[test]
    fn directory_scroll_areas_have_distinct_ids_when_rendered() {
        let database = finance_db::Database::open_in_memory().expect("in-memory SQLite should open");
        database
            .execute_batch(include_str!("../../db/migrations/V1__initial.sql"))
            .expect("initial migration should apply");
        database
            .execute_batch(include_str!("../../db/migrations/V2__transactions_and_directories.sql"))
            .expect("directory migration should apply");
        database
            .execute("INSERT INTO income_categories (name) VALUES ('Тестовая категория')", [])
            .expect("test category should be inserted");
        database
            .execute("INSERT INTO banks (name) VALUES ('Тестовый банк')", [])
            .expect("test bank should be inserted");

        let mut app = FinanceApp::new(database);
        app.active_tab = ModuleTab::Directories;
        app.refresh_directories();

        let context = egui::Context::default();
        let mut scroll_area_ids = None;
        let _output = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                scroll_area_ids = Some(app.render_directories(ui));
            });
        });

        let (categories_id, banks_id) =
            scroll_area_ids.expect("directory screen should render both lists");
        assert_ne!(categories_id, banks_id);
        assert_ne!(DIRECTORY_CATEGORIES_SCROLL_AREA_ID, DIRECTORY_BANKS_SCROLL_AREA_ID);
    }
}