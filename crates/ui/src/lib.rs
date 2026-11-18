use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    parse_amount_expression, Bank, Category, Currency, FinancialTransaction, TransactionKind,
};
use finance_db::{self, Database};
use rust_decimal::Decimal;

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
    category_filter: Option<i64>,
    show_transaction_form: bool,
    show_category_form: bool,
    show_bank_form: bool,
    pending_delete: Option<DeleteTarget>,
    message: Option<(String, bool)>,
}

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
        ui.horizontal(|ui| {
            if ui.button("＋ Добавить").clicked() {
                self.refresh_categories(kind, true);
                self.start_new_transaction();
            }
            if ui.button("Обновить").clicked() {
                self.refresh_transactions(kind);
            }
            ui.separator();
            ui.label("Поиск");
            ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(150.0));
            ui.label("С");
            ui.add(
                egui::TextEdit::singleline(&mut self.from_date)
                    .desired_width(95.0)
                    .hint_text("ГГГГ-ММ-ДД"),
            );
            ui.label("По");
            ui.add(
                egui::TextEdit::singleline(&mut self.to_date)
                    .desired_width(95.0)
                    .hint_text("ГГГГ-ММ-ДД"),
            );
            if ui.button("Применить").clicked() {
                self.refresh_transactions(kind);
            }
        });

        ui.horizontal(|ui| {
            ui.label("Категория:");
            let selected_name = self.category_filter
                .and_then(|id| self.categories.iter().find(|category| category.id == id))
                .map_or("Все", |category| category.name.as_str());
            egui::ComboBox::from_id_salt("transaction_category_filter")
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
            if !self.categories.iter().any(|category| category.is_active) {
                ui.label("Сначала добавьте категорию в разделе «Справочники».");
            }
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("transactions_table")
                .striped(true)
                .min_col_width(85.0)
                .show(ui, |ui| {
                    for heading in [
                        "Дата",
                        "Категория",
                        "Сумма",
                        "Валюта",
                        "Комментарий",
                        "Действия",
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    let transactions = self.transactions.clone();
                    for transaction in &transactions {
                        ui.label(transaction.date.format("%d.%m.%Y").to_string());
                        ui.label(&transaction.category_name);
                        ui.label(transaction.amount.to_string());
                        ui.label(transaction.currency.code());
                        ui.label(&transaction.comment);
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

        if self.show_transaction_form {
            self.render_transaction_form(ui, kind);
        }
    }

    fn render_transaction_form(&mut self, ui: &mut egui::Ui, kind: TransactionKind) {
        ui.separator();
        ui.heading(if self.transaction_form.id.is_some() { "Изменить операцию" } else { "Новая операция" });
        ui.horizontal(|ui| {
            ui.label("Дата (ГГГГ-ММ-ДД)");
            ui.text_edit_singleline(&mut self.transaction_form.date);
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
            ui.label("Комментарий");
            ui.add(egui::TextEdit::singleline(&mut self.transaction_form.comment).desired_width(240.0));
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

    fn render_directories(&mut self, ui: &mut egui::Ui) {
        let previous_kind = self.directory_kind;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.directory_kind, TransactionKind::Income, "Категории доходов");
            ui.selectable_value(&mut self.directory_kind, TransactionKind::Expense, "Категории расходов");
            if ui.button("Обновить").clicked() {
                self.refresh_directories();
            }
        });
        if self.directory_kind != previous_kind {
            self.category_form = CategoryForm::default();
            self.show_category_form = false;
            self.refresh_categories(self.directory_kind, true);
        }
        ui.columns(2, |columns| {
            columns[0].heading("Категории");
            columns[0].horizontal(|ui| {
                if ui.button("＋ Добавить категорию").clicked() {
                    self.category_form = CategoryForm { is_active: true, ..CategoryForm::default() };
                    self.show_category_form = true;
                }
            });
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                let categories = self.categories.clone();
                for category in categories {
                    ui.horizontal(|ui| {
                        let prefix = if category.parent_id.is_some() { "　↳ " } else { "" };
                        ui.label(format!("{prefix}{}{}", category.name, if category.is_active { "" } else { " (неактивна)" }));
                        if ui.small_button("Изменить").clicked() {
                            self.category_form = CategoryForm {
                                id: Some(category.id),
                                name: category.name.clone(),
                                parent_id: category.parent_id,
                                is_active: category.is_active,
                            };
                            self.show_category_form = true;
                        }
                        if ui.small_button("Удалить").clicked() {
                            self.pending_delete = Some(DeleteTarget::Category(self.directory_kind, category.id));
                        }
                    });
                }
            });
            columns[1].heading("Банки");
            columns[1].horizontal(|ui| {
                if ui.button("＋ Добавить банк").clicked() {
                    self.bank_form = BankForm::default();
                    self.show_bank_form = true;
                }
            });
            egui::ScrollArea::vertical().show(&mut columns[1], |ui| {
                let banks = self.banks.clone();
                for bank in banks {
                    ui.horizontal(|ui| {
                        ui.label(&bank.name);
                        if ui.small_button("Изменить").clicked() {
                            self.bank_form = BankForm {
                                id: Some(bank.id),
                                name: bank.name.clone(),
                                comment: bank.comment.clone(),
                            };
                            self.show_bank_form = true;
                        }
                        if ui.small_button("Удалить").clicked() {
                            self.pending_delete = Some(DeleteTarget::Bank(bank.id));
                        }
                    });
                }
            });
        });

        if self.show_category_form {
            self.render_category_form(ui);
        }
        if self.show_bank_form {
            self.render_bank_form(ui);
        }
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

        egui::CentralPanel::default().show(context, |ui| {
            ui.heading(self.active_tab.title());
            ui.separator();
            match self.active_tab {
                ModuleTab::Income => self.render_transactions(ui, TransactionKind::Income),
                ModuleTab::Expenses => self.render_transactions(ui, TransactionKind::Expense),
                ModuleTab::Directories => self.render_directories(ui),
                ModuleTab::Savings | ModuleTab::PaymentDocuments | ModuleTab::Analysis => {
                    ui.label("Этот раздел пока не реализован.");
                }
            }
            if let Some((message, is_error)) = &self.message {
                ui.separator();
                ui.colored_label(if *is_error { egui::Color32::RED } else { egui::Color32::DARK_GREEN }, message);
            }
        });
        self.render_delete_confirmation(context);
    }
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