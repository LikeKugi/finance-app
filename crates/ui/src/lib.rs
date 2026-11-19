use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;

use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    parse_amount_expression, summarize_transaction_balance, Bank, Category, Currency,
    CurrencyDefinition, FinancialTransaction, PaymentCardType, PaymentDocument, TransactionBalance,
    TransactionKind,
};
use finance_db::{self, Database};
use rust_decimal::Decimal;

const TABLE_ROW_HEIGHT: f32 = 48.0;
const DIVISION_ROUNDING_WARNING: &str = "Деление округляется до 2 знаков после запятой; погрешность может накапливаться при нескольких делениях.";
const TRANSACTION_COLUMN_MIN_WIDTHS: [f32; 6] = [115.0, 135.0, 90.0, 180.0, 220.0, 180.0];
const TRANSACTION_COLUMN_GROWTH_WEIGHTS: [f32; 6] = [0.8, 0.8, 0.6, 1.1, 1.8, 1.0];

fn transaction_column_widths(available_width: f32) -> [f32; 6] {
    let minimum_width: f32 = TRANSACTION_COLUMN_MIN_WIDTHS.iter().sum();
    let extra_width = (available_width - minimum_width).max(0.0);
    let total_growth_weight: f32 = TRANSACTION_COLUMN_GROWTH_WEIGHTS.iter().sum();

    std::array::from_fn(|index| {
        TRANSACTION_COLUMN_MIN_WIDTHS[index]
            + extra_width * TRANSACTION_COLUMN_GROWTH_WEIGHTS[index] / total_growth_weight
    })
}

fn render_analysis_footer(
    context: &egui::Context,
    active_tab: ModuleTab,
    analysis_page: &AnalysisPage,
) {
    if active_tab == ModuleTab::Analysis {
        egui::TopBottomPanel::bottom("analysis_footer").show(context, |ui| {
            ui.separator();
            analysis_page.render_footer(ui);
        });
    }
}

fn render_transaction_balance_footer(
    ui: &mut egui::Ui,
    balances: Option<&[TransactionBalance; 2]>,
    period: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
) {
    match (balances, period) {
        (Some(balances), Some((from, to))) => {
            let period_label = transaction_balance_period_label(from, to);
            for balance in balances {
                ui.label(
                    egui::RichText::new(format!(
                        "Доходы {period_label}: {} {} − Расходы {period_label}: {} {} = Сальдо {period_label}: {} {}",
                        balance.income,
                        balance.currency.code(),
                        balance.expenses,
                        balance.currency.code(),
                        balance.balance,
                        balance.currency.code(),
                    ))
                    .size(egui::TextStyle::Body.resolve(ui.style()).size + 4.0)
                    .strong(),
                );
            }
            ui.weak(
                "Сальдо операций не включает начальные остатки и переводы и не подтверждает сумму денег на руках.",
            );
        }
        _ => {
            ui.colored_label(
                egui::Color32::RED,
                "Сальдо не рассчитано: обновите список после устранения ошибки.",
            );
        }
    }
}

mod analysis_page;
mod date_input;
mod payment_documents_page;
mod savings_page;

use analysis_page::AnalysisPage;
use date_input::{date_input, DateInputState};
use payment_documents_page::PaymentDocumentsPage;
use savings_page::SavingsPage;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectoryRightTab {
    Banks,
    Currencies,
}

#[derive(Clone)]
enum DeleteTarget {
    Transaction(TransactionKind, i64),
    Category(TransactionKind, i64),
    Bank(i64),
    Currency(String),
}

#[derive(Default)]
struct TransactionForm {
    id: Option<i64>,
    date: String,
    category_id: Option<i64>,
    amount: String,
    currency: Option<Currency>,
    comment: String,
    payment_document_id: Option<i64>,
    affects_deposit_rate: bool,
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

#[derive(Default)]
struct CurrencyForm {
    code: String,
    name: String,
    rate: String,
    rate_date: String,
    editing_rate: bool,
}

struct FinanceApp {
    database: Database,
    database_path: Option<PathBuf>,
    active_tab: ModuleTab,
    transactions: Vec<FinancialTransaction>,
    transaction_balances: Option<[TransactionBalance; 2]>,
    transaction_balance_period: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
    categories: Vec<Category>,
    analysis_income_categories: Vec<Category>,
    analysis_expense_categories: Vec<Category>,
    banks: Vec<Bank>,
    currencies: Vec<CurrencyDefinition>,
    used_currencies: HashSet<String>,
    transaction_form: TransactionForm,
    transaction_cards: Vec<PaymentDocument>,
    category_form: CategoryForm,
    bank_form: BankForm,
    currency_form: CurrencyForm,
    savings_page: SavingsPage,
    payment_documents_page: PaymentDocumentsPage,
    analysis_page: AnalysisPage,
    directory_kind: TransactionKind,
    directory_right_tab: DirectoryRightTab,
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
    show_currency_form: bool,
    pending_delete: Option<DeleteTarget>,
    message: Option<(String, bool)>,
}

const TRANSACTION_FILTERS_SCROLL_AREA_ID: &str = "transaction_filters_scroll_area";
const TRANSACTION_TABLE_SCROLL_AREA_ID: &str = "transaction_table_scroll_area";
const DIRECTORY_CATEGORIES_SCROLL_AREA_ID: &str = "directory_categories_scroll_area";
const DIRECTORY_BANKS_SCROLL_AREA_ID: &str = "directory_banks_scroll_area";

impl FinanceApp {
    #[cfg(test)]
    fn new(database: Database) -> Self {
        Self::with_database_path(database, None)
    }

    fn with_database_path(database: Database, database_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            database,
            database_path,
            active_tab: ModuleTab::Income,
            transactions: Vec::new(),
            transaction_balances: None,
            transaction_balance_period: None,
            categories: Vec::new(),
            analysis_income_categories: Vec::new(),
            analysis_expense_categories: Vec::new(),
            banks: Vec::new(),
            currencies: Vec::new(),
            used_currencies: HashSet::new(),
            transaction_form: TransactionForm::default(),
            transaction_cards: Vec::new(),
            category_form: CategoryForm::default(),
            bank_form: BankForm::default(),
            currency_form: CurrencyForm::default(),
            savings_page: SavingsPage::default(),
            payment_documents_page: PaymentDocumentsPage::default(),
            analysis_page: AnalysisPage::default(),
            directory_kind: TransactionKind::Income,
            directory_right_tab: DirectoryRightTab::Banks,
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
            show_currency_form: false,
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
        if self.active_tab == ModuleTab::Savings {
            self.refresh_savings();
        }
        if self.active_tab == ModuleTab::PaymentDocuments {
            self.refresh_payment_documents();
        }
        if self.active_tab == ModuleTab::Directories {
            self.refresh_directories();
        }
        if self.active_tab == ModuleTab::Analysis {
            self.refresh_analysis_categories();
        }
    }

    fn refresh_savings(&mut self) {
        match finance_db::list_banks(&self.database) {
            Ok(banks) => self.banks = banks,
            Err(error) => {
                self.show_error(error.to_string());
                return;
            }
        }
        if let Err(error) = self.savings_page.refresh(&self.database) {
            self.show_error(error.to_string());
        }
    }

    fn refresh_payment_documents(&mut self) {
        match finance_db::list_banks(&self.database) {
            Ok(banks) => self.banks = banks,
            Err(error) => {
                self.show_error(error.to_string());
                return;
            }
        }
        if let Err(error) = self.payment_documents_page.refresh(&self.database) {
            self.show_error(error.to_string());
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
            self.message = Some((
                "Начало периода не может быть позже окончания".to_owned(),
                true,
            ));
            return;
        }
        let transactions = match finance_db::list_transactions(
            &self.database,
            kind,
            from,
            to,
            self.category_filter,
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
                self.message = None;
            }
            Err(error) => {
                self.transaction_balances = None;
                self.transaction_balance_period = None;
                self.show_error(error.to_string());
            }
        }
    }

    fn refresh_categories(&mut self, kind: TransactionKind, include_inactive: bool) {
        match finance_db::list_categories(&self.database, kind, include_inactive) {
            Ok(categories) => self.categories = categories,
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn refresh_analysis_categories(&mut self) {
        let income_categories =
            finance_db::list_categories(&self.database, TransactionKind::Income, true);
        let expense_categories =
            finance_db::list_categories(&self.database, TransactionKind::Expense, true);

        match (income_categories, expense_categories) {
            (Ok(income_categories), Ok(expense_categories)) => {
                self.analysis_income_categories = income_categories;
                self.analysis_expense_categories = expense_categories;
            }
            (Err(error), _) | (_, Err(error)) => self.show_error(error.to_string()),
        }
    }

    fn refresh_directories(&mut self) {
        self.refresh_categories(self.directory_kind, true);
        match finance_db::list_banks(&self.database) {
            Ok(banks) => self.banks = banks,
            Err(error) => self.show_error(error.to_string()),
        }
        match finance_db::list_currencies(&self.database) {
            Ok(currencies) => self.currencies = currencies,
            Err(error) => self.show_error(error.to_string()),
        }
        match finance_db::list_used_currency_codes(&self.database) {
            Ok(currencies) => self.used_currencies = currencies.into_iter().collect(),
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

    fn edit_transaction(&mut self, transaction: &FinancialTransaction) {
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

    fn load_transaction_cards(&mut self, kind: TransactionKind) -> bool {
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

    fn save_currency(&mut self) {
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

    fn edit_currency_rate(&mut self, currency: &CurrencyDefinition) {
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

    fn change_primary_currency(&mut self, code: &str) {
        match finance_db::set_primary_currency(&mut self.database, code) {
            Ok(()) => {
                self.show_success("Основная валюта изменена; суммы операций не пересчитаны");
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
            DeleteTarget::Category(kind, id) => {
                finance_db::delete_category(&self.database, kind, id)
            }
            DeleteTarget::Bank(id) => finance_db::delete_bank(&self.database, id),
            DeleteTarget::Currency(code) => finance_db::delete_currency(&self.database, &code),
        };
        match result {
            Ok(()) => {
                self.show_success("Запись удалена");
                self.refresh_all();
            }
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn clear_search_filter(&mut self, kind: TransactionKind) {
        self.search.clear();
        self.refresh_transactions(kind);
    }

    fn clear_from_date_filter(&mut self, kind: TransactionKind) {
        self.from_date.clear();
        self.from_date_input.reset();
        self.refresh_transactions(kind);
    }

    fn clear_to_date_filter(&mut self, kind: TransactionKind) {
        self.to_date.clear();
        self.to_date_input.reset();
        self.refresh_transactions(kind);
    }

    fn clear_category_filter(&mut self, kind: TransactionKind) {
        self.category_filter = None;
        self.refresh_transactions(kind);
    }

    fn reset_transaction_filters(&mut self, kind: TransactionKind) {
        self.search.clear();
        self.from_date.clear();
        self.to_date.clear();
        self.from_date_input.reset();
        self.to_date_input.reset();
        self.category_filter = None;
        self.refresh_transactions(kind);
    }

    fn render_transactions(&mut self, ui: &mut egui::Ui, kind: TransactionKind) {
        egui::ScrollArea::horizontal()
            .id_salt(TRANSACTION_FILTERS_SCROLL_AREA_ID)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Поиск");
                    ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(150.0));
                    if ui
                        .add_enabled(!self.search.is_empty(), egui::Button::new("×"))
                        .on_hover_text("Очистить поиск")
                        .clicked()
                    {
                        self.clear_search_filter(kind);
                    }
                    ui.add_space(8.0);
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
                    ui.add_space(8.0);
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
                    ui.add_space(8.0);
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
                    ui.add_space(8.0);
                    if ui.button("Применить").clicked() {
                        self.refresh_transactions(kind);
                    }
                    let has_active_filters = !self.search.is_empty()
                        || !self.from_date.is_empty()
                        || !self.to_date.is_empty()
                        || self.category_filter.is_some();
                    if ui
                        .add_enabled(has_active_filters, egui::Button::new("Сбросить фильтры"))
                        .clicked()
                    {
                        self.reset_transaction_filters(kind);
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

    fn render_transaction_form(&mut self, ui: &mut egui::Ui, kind: TransactionKind) {
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
            ui.weak("Доступны дебетовые карты с указанной валютой. Для старых карт сначала укажите валюту в платёжных документах.");
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

    fn render_directories(&mut self, ui: &mut egui::Ui) -> (egui::Id, egui::Id) {
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

    fn render_bank_directory(&mut self, ui: &mut egui::Ui) {
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

    fn render_currency_directory(&mut self, ui: &mut egui::Ui) {
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

    fn render_category_form(&mut self, ui: &mut egui::Ui) {
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

    fn render_bank_form(&mut self, ui: &mut egui::Ui) {
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

    fn render_currency_form(&mut self, ui: &mut egui::Ui) {
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

    fn render_delete_confirmation(&mut self, context: &egui::Context) {
        let Some(target) = self.pending_delete.clone() else {
            return;
        };
        let confirmation_text = match &target {
            DeleteTarget::Currency(code) => format!(
                "Удалить валюту {code}? Основную валюту и валюты, используемые записями, удалить нельзя."
            ),
            _ => "Удалить выбранную запись? Это действие нельзя отменить.".to_owned(),
        };
        egui::Window::new("Подтверждение удаления")
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(confirmation_text);
                ui.horizontal(|ui| {
                    if ui.button("Удалить").clicked() {
                        self.pending_delete = Some(target.clone());
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
                    if ui
                        .selectable_value(&mut self.active_tab, tab, tab.title())
                        .clicked()
                    {
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
                render_transaction_balance_footer(
                    ui,
                    self.transaction_balances.as_ref(),
                    self.transaction_balance_period,
                );
            });
        }

        if self.active_tab == ModuleTab::Savings {
            egui::TopBottomPanel::bottom("savings_footer").show(context, |ui| {
                ui.separator();
                self.savings_page.render_footer(ui);
            });
        }
        render_analysis_footer(context, self.active_tab, &self.analysis_page);

        let mut refresh_analysis_categories = false;
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
                } else if matches!(
                    self.active_tab,
                    ModuleTab::Directories | ModuleTab::Analysis
                ) {
                    let actions_width = 140.0;
                    ui.add_space((ui.available_width() - actions_width).max(0.0));
                    if ui
                        .add_sized(egui::vec2(140.0, 40.0), egui::Button::new("↻ Обновить"))
                        .clicked()
                    {
                        if self.active_tab == ModuleTab::Directories {
                            self.refresh_directories();
                        } else {
                            refresh_analysis_categories = true;
                        }
                    }
                } else if self.active_tab == ModuleTab::Savings {
                    let actions_width = 140.0 * 2.0 + ui.spacing().item_spacing.x;
                    ui.add_space((ui.available_width() - actions_width).max(0.0));
                    if ui
                        .add_enabled_ui(!self.banks.is_empty(), |ui| {
                            ui.add_sized(egui::vec2(140.0, 40.0), egui::Button::new("＋ Добавить"))
                        })
                        .inner
                        .clicked()
                    {
                        self.savings_page.begin_new(&self.banks);
                    }
                    if ui
                        .add_sized(egui::vec2(140.0, 40.0), egui::Button::new("↻ Обновить"))
                        .clicked()
                    {
                        self.refresh_savings();
                    }
                } else if self.active_tab == ModuleTab::PaymentDocuments {
                    let actions_width = 140.0 * 2.0 + ui.spacing().item_spacing.x;
                    ui.add_space((ui.available_width() - actions_width).max(0.0));
                    if ui
                        .add_enabled_ui(!self.banks.is_empty(), |ui| {
                            ui.add_sized(egui::vec2(140.0, 40.0), egui::Button::new("＋ Добавить"))
                        })
                        .inner
                        .clicked()
                    {
                        if let Err(error) = self
                            .payment_documents_page
                            .begin_new(&self.database, self.banks.first().map(|bank| bank.id))
                        {
                            self.show_error(error.to_string());
                        }
                    }
                    if ui
                        .add_sized(egui::vec2(140.0, 40.0), egui::Button::new("↻ Обновить"))
                        .clicked()
                    {
                        self.refresh_payment_documents();
                    }
                }
            });
            ui.separator();
            if let Some((message, is_error)) = &self.message {
                ui.colored_label(
                    if *is_error {
                        egui::Color32::RED
                    } else {
                        egui::Color32::DARK_GREEN
                    },
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
                ModuleTab::Savings => {
                    if let Some(message) =
                        self.savings_page
                            .render(context, ui, &self.database, &self.banks)
                    {
                        self.message = Some(message);
                    }
                }
                ModuleTab::PaymentDocuments => {
                    if let Some(message) =
                        self.payment_documents_page
                            .render(context, ui, &self.database, &self.banks)
                    {
                        self.message = Some(message);
                    }
                }
                ModuleTab::Analysis => {
                    refresh_analysis_categories = self.analysis_page.render(
                        context,
                        ui,
                        self.database_path.as_deref(),
                        &self.analysis_income_categories,
                        &self.analysis_expense_categories,
                        refresh_analysis_categories,
                    );
                }
            }
        });
        if refresh_analysis_categories {
            self.refresh_analysis_categories();
        }
        self.render_delete_confirmation(context);
    }
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

fn category_tree_labels(categories: &[Category]) -> Vec<(i64, String)> {
    let mut path = Vec::new();

    category_tree_rows(categories)
        .into_iter()
        .map(|(index, depth)| {
            path.truncate(depth);
            path.push(categories[index].name.as_str());
            (categories[index].id, path.join(" → "))
        })
        .collect()
}

fn currency_rate_label(currency: &CurrencyDefinition, primary_code: &str) -> String {
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

fn transaction_balance_period_label(from: Option<NaiveDate>, to: Option<NaiveDate>) -> String {
    match (from, to) {
        (None, None) => "за весь период".to_owned(),
        (Some(from), Some(to)) => format!(
            "за период с {} по {}",
            from.format("%d.%m.%Y"),
            to.format("%d.%m.%Y")
        ),
        (Some(from), None) => format!("за период с {}", from.format("%d.%m.%Y")),
        (None, Some(to)) => format!("за период по {}", to.format("%d.%m.%Y")),
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
pub fn run(database: Database, database_path: PathBuf) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([1280.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "FinanceApp — управление финансами",
        options,
        Box::new(move |_creation_context| {
            Ok(Box::new(FinanceApp::with_database_path(
                database,
                Some(database_path),
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use eframe::egui;
    use finance_core::{Currency, TransactionBalance};

    use super::{
        category_tree_labels, category_tree_rows, currency_rate_label, render_analysis_footer,
        render_transaction_balance_footer, transaction_column_widths, AnalysisPage, Category,
        CurrencyForm, DirectoryRightTab, FinanceApp, ModuleTab, SavingsPage, TransactionKind,
        DIRECTORY_BANKS_SCROLL_AREA_ID, DIRECTORY_CATEGORIES_SCROLL_AREA_ID,
        DIVISION_ROUNDING_WARNING, TABLE_ROW_HEIGHT, TRANSACTION_COLUMN_MIN_WIDTHS,
        TRANSACTION_FILTERS_SCROLL_AREA_ID, TRANSACTION_TABLE_SCROLL_AREA_ID,
    };

    fn category(id: i64, parent_id: Option<i64>, is_active: bool) -> Category {
        Category {
            id,
            name: format!("Категория {id}"),
            parent_id,
            is_active,
        }
    }

    fn filtered_transaction_app(kind: TransactionKind) -> FinanceApp {
        let database =
            finance_db::Database::open_in_memory().expect("in-memory SQLite should open");
        database
            .execute_batch(include_str!("../../db/migrations/V1__initial.sql"))
            .expect("initial migration should apply");
        database
            .execute_batch(include_str!(
                "../../db/migrations/V2__transactions_and_directories.sql"
            ))
            .expect("transaction migration should apply");
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

        let (category_table, transaction_table, active_tab) = match kind {
            TransactionKind::Income => ("income_categories", "incomes", ModuleTab::Income),
            TransactionKind::Expense => ("expense_categories", "expenses", ModuleTab::Expenses),
        };
        database
            .execute_batch(&format!(
                "INSERT INTO {category_table} (id, name) VALUES
                    (1, 'Основная'),
                    (2, 'Другая');
                INSERT INTO {transaction_table} (id, date, category_id, amount, currency, comment) VALUES
                    (1, '2023-12-15', 1, '10', 'RUB', 'найти'),
                    (2, '2024-01-15', 1, '20', 'RUB', 'найти'),
                    (3, '2024-01-15', 1, '30', 'RUB', 'пропустить'),
                    (4, '2024-01-15', 2, '40', 'RUB', 'найти'),
                    (5, '2024-02-15', 1, '50', 'RUB', 'найти');"
            ))
            .expect("transaction fixtures should be inserted");

        let mut app = FinanceApp::new(database);
        app.active_tab = active_tab;
        app.refresh_categories(kind, true);
        app.search = "найти".to_owned();
        app.from_date = "2024-01-01".to_owned();
        app.to_date = "2024-01-31".to_owned();
        app.category_filter = Some(1);
        app.refresh_transactions(kind);
        assert_eq!(
            app.transactions.len(),
            1,
            "fixture should match all filters"
        );
        app
    }

    fn directory_app() -> FinanceApp {
        let database = finance_db::Database::open_in_memory().unwrap();
        for migration in [
            include_str!("../../db/migrations/V1__initial.sql"),
            include_str!("../../db/migrations/V2__transactions_and_directories.sql"),
            include_str!("../../db/migrations/V3__savings.sql"),
            include_str!("../../db/migrations/V4__payment_documents.sql"),
            include_str!("../../db/migrations/V5__savings_dates.sql"),
            include_str!("../../db/migrations/V6__income_card_balances.sql"),
            include_str!("../../db/migrations/V7__expense_cards.sql"),
            include_str!("../../db/migrations/V8__currencies.sql"),
        ] {
            database.execute_batch(migration).unwrap();
        }

        let mut app = FinanceApp::new(database);
        app.active_tab = ModuleTab::Directories;
        app.refresh_directories();
        app
    }

    #[test]
    fn income_form_credits_card_and_preserves_selection_when_editing() {
        let mut app = filtered_transaction_app(TransactionKind::Income);
        let bank = finance_db::save_bank(&app.database, None, "Банк", "").unwrap();
        let card = finance_db::save_payment_document(
            &app.database,
            None,
            &finance_core::PaymentDocumentDetails {
                bank_id: bank,
                payment_system: finance_core::PaymentSystem::Mir,
                card_type: finance_core::PaymentCardType::Debit,
                amount: 0.into(),
                currency: Some(Currency::Rub),
                due_date: None,
                grace_period_days: None,
                affects_deposit_rate: false,
                min_monthly_spend: None,
                savings_ids: Vec::new(),
            },
        )
        .unwrap();
        app.start_new_transaction();
        app.transaction_form.date = "2024-01-20".to_owned();
        app.transaction_form.category_id = Some(1);
        app.transaction_form.amount = "12.50".to_owned();
        app.transaction_form.comment = "найти".to_owned();
        app.transaction_form.payment_document_id = Some(card);
        app.save_transaction(TransactionKind::Income);
        assert!(!app.show_transaction_form);
        let saved = app
            .transactions
            .iter()
            .find(|transaction| transaction.payment_document_id == Some(card))
            .unwrap()
            .clone();
        assert_eq!(
            finance_db::list_payment_documents(&app.database, None, None, None).unwrap()[0]
                .amount
                .to_string(),
            "12.5"
        );
        app.edit_transaction(&saved);
        assert_eq!(app.transaction_form.payment_document_id, Some(card));
        app.transaction_form.amount = "15".to_owned();
        app.save_transaction(TransactionKind::Income);
        assert_eq!(
            finance_db::list_payment_documents(&app.database, None, None, None).unwrap()[0]
                .amount
                .to_string(),
            "15"
        );
    }

    #[test]
    fn expense_form_saves_card_and_rate_flag_and_restores_them_for_editing() {
        let mut app = filtered_transaction_app(TransactionKind::Expense);
        let bank = finance_db::save_bank(&app.database, None, "Банк", "").unwrap();
        let savings = finance_db::save_savings(
            &app.database,
            None,
            &finance_core::SavingsDetails {
                bank_id: bank,
                title: "Резерв".to_owned(),
                amount: 1000.into(),
                base_rate: 5.into(),
                rate_after_threshold: None,
                threshold_amount: None,
                opened_date: None,
                interest_accrual_date: None,
                term_type: finance_core::SavingsTerm::Unlimited,
                term_end_date: None,
                interest_destination: finance_core::InterestDestination::Withdraw,
                is_active: true,
            },
        )
        .unwrap();
        let card = finance_db::save_payment_document(
            &app.database,
            None,
            &finance_core::PaymentDocumentDetails {
                bank_id: bank,
                payment_system: finance_core::PaymentSystem::Mir,
                card_type: finance_core::PaymentCardType::Credit,
                amount: 5000.into(),
                currency: Some(Currency::Rub),
                due_date: NaiveDate::from_ymd_opt(2024, 2, 1),
                grace_period_days: Some(30),
                affects_deposit_rate: true,
                min_monthly_spend: Some(1000.into()),
                savings_ids: vec![savings],
            },
        )
        .unwrap();

        app.start_new_transaction();
        app.transaction_form.date = "2024-01-20".to_owned();
        app.transaction_form.category_id = Some(1);
        app.transaction_form.amount = "12.50".to_owned();
        app.transaction_form.comment = "найти расход по карте".to_owned();
        app.transaction_form.payment_document_id = Some(card);
        app.transaction_form.affects_deposit_rate = true;
        app.save_transaction(TransactionKind::Expense);

        let saved = app
            .transactions
            .iter()
            .find(|transaction| transaction.payment_document_id == Some(card))
            .unwrap()
            .clone();
        assert!(saved.affects_deposit_rate);
        app.edit_transaction(&saved);
        assert_eq!(app.transaction_form.payment_document_id, Some(card));
        assert!(app.transaction_form.affects_deposit_rate);

        app.transaction_form.affects_deposit_rate = false;
        app.save_transaction(TransactionKind::Expense);
        let updated = finance_db::list_transactions(
            &app.database,
            TransactionKind::Expense,
            None,
            None,
            None,
            "",
        )
        .unwrap()
        .into_iter()
        .find(|transaction| transaction.payment_document_id == Some(card))
        .unwrap();
        assert!(!updated.affects_deposit_rate);
    }

    #[test]
    fn transaction_form_rejects_card_currency_mismatch_after_currency_changes() {
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            let mut app = filtered_transaction_app(kind);
            let bank = finance_db::save_bank(&app.database, None, "Банк", "").unwrap();
            let card = finance_db::save_payment_document(
                &app.database,
                None,
                &finance_core::PaymentDocumentDetails {
                    bank_id: bank,
                    payment_system: finance_core::PaymentSystem::Mir,
                    card_type: finance_core::PaymentCardType::Debit,
                    amount: 0.into(),
                    currency: Some(Currency::Usd),
                    due_date: None,
                    grace_period_days: None,
                    affects_deposit_rate: false,
                    min_monthly_spend: None,
                    savings_ids: Vec::new(),
                },
            )
            .unwrap();
            let transaction_count =
                finance_db::list_transactions(&app.database, kind, None, None, None, "")
                    .unwrap()
                    .len();

            app.start_new_transaction();
            app.transaction_form.date = "2024-01-20".to_owned();
            app.transaction_form.category_id = Some(1);
            app.transaction_form.amount = "12.50".to_owned();
            app.transaction_form.currency = Some(Currency::Usd);
            app.transaction_form.payment_document_id = Some(card);
            app.transaction_form.currency = Some(Currency::Rub);
            app.save_transaction(kind);

            assert!(app.show_transaction_form);
            assert!(app
                .message
                .as_ref()
                .is_some_and(|(message, is_error)| *is_error && message.contains("валют")));
            assert_eq!(
                finance_db::list_transactions(&app.database, kind, None, None, None, "")
                    .unwrap()
                    .len(),
                transaction_count
            );
        }
    }

    #[test]
    fn transaction_balance_uses_only_applied_dates_on_both_transaction_pages() {
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            let mut app = filtered_transaction_app(kind);
            let balances = app
                .transaction_balances
                .expect("a successful refresh should calculate both currency balances");

            match kind {
                TransactionKind::Income => assert_eq!(balances[0].income, 90.into()),
                TransactionKind::Expense => assert_eq!(balances[0].expenses, 90.into()),
            }
            assert_eq!(app.transactions.len(), 1);
            assert_eq!(
                app.transaction_balance_period,
                Some((
                    NaiveDate::from_ymd_opt(2024, 1, 1),
                    NaiveDate::from_ymd_opt(2024, 1, 31)
                ))
            );

            app.from_date = "2024-02-01".to_owned();

            assert_eq!(app.transaction_balances, Some(balances));
            assert_eq!(
                app.transaction_balance_period,
                Some((
                    NaiveDate::from_ymd_opt(2024, 1, 1),
                    NaiveDate::from_ymd_opt(2024, 1, 31)
                ))
            );
        }
    }

    #[test]
    fn failed_transaction_refresh_does_not_keep_stale_balance() {
        let mut app = filtered_transaction_app(TransactionKind::Income);
        app.database
            .execute_batch("DROP TABLE incomes;")
            .expect("test should remove the income table");

        app.refresh_transactions(TransactionKind::Income);

        assert!(app.transaction_balances.is_none());
        assert!(app.transaction_balance_period.is_none());
    }

    #[test]
    fn transaction_balance_footer_shows_formula_for_income_and_expenses_tabs() {
        let balances = [
            TransactionBalance {
                currency: Currency::Rub,
                income: 100.into(),
                expenses: 25.into(),
                balance: 75.into(),
            },
            TransactionBalance {
                currency: Currency::Usd,
                income: 20.into(),
                expenses: 5.into(),
                balance: 15.into(),
            },
        ];

        for active_tab in [ModuleTab::Income, ModuleTab::Expenses] {
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
                        if active_tab.transaction_kind().is_some() {
                            egui::TopBottomPanel::bottom("transaction_balance_footer").show(
                                context,
                                |ui| {
                                    render_transaction_balance_footer(
                                        ui,
                                        Some(&balances),
                                        Some((None, None)),
                                    );
                                },
                            );
                        }
                    },
                ));
            }
            let output = output.expect("balance footer should render twice");

            assert_eq!(
                text_positions(
                    &output,
                    "Доходы за весь период: 100 RUB − Расходы за весь период: 25 RUB = Сальдо за весь период: 75 RUB"
                )
                .len(),
                1
            );
            let rendered_text = output
                .shapes
                .iter()
                .filter_map(|clipped_shape| match &clipped_shape.shape {
                    egui::epaint::Shape::Text(text_shape) => {
                        Some(text_shape.galley.job.text.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                rendered_text
                    .iter()
                    .any(|text| text.contains("Сальдо операций не включает")),
                "rendered labels: {rendered_text:?}"
            );
        }
    }

    fn transaction_rows(app: &FinanceApp) -> Vec<(String, i64, String)> {
        app.transactions
            .iter()
            .map(|transaction| {
                (
                    transaction.date.format("%Y-%m-%d").to_string(),
                    transaction.category_id,
                    transaction.comment.clone(),
                )
            })
            .collect()
    }

    fn render_transactions_at_width(
        app: &mut FinanceApp,
        context: &egui::Context,
        kind: TransactionKind,
        width: f32,
    ) -> egui::FullOutput {
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 720.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        app.render_transactions(ui, kind);
                    });
                },
            ));
        }
        output.expect("transaction screen should render twice")
    }

    fn render_transaction_form_at_width(
        app: &mut FinanceApp,
        context: &egui::Context,
        kind: TransactionKind,
        width: f32,
    ) -> egui::FullOutput {
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 720.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        app.render_transaction_form(ui, kind);
                    });
                },
            ));
        }
        output.expect("transaction form should render twice")
    }

    fn render_directories_at_width(
        app: &mut FinanceApp,
        context: &egui::Context,
        width: f32,
    ) -> egui::FullOutput {
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 720.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        app.render_directories(ui);
                    });
                },
            ));
        }
        output.expect("directory screen should render twice")
    }

    fn text_positions(output: &egui::FullOutput, text: &str) -> Vec<egui::Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) if text_shape.galley.job.text == text => {
                    Some(text_shape.pos)
                }
                _ => None,
            })
            .collect()
    }

    fn render_analysis_footer_at_width(active_tab: ModuleTab, width: f32) -> egui::FullOutput {
        let context = egui::Context::default();
        let analysis_page = AnalysisPage::default();
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 720.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    render_analysis_footer(context, active_tab, &analysis_page);
                    egui::CentralPanel::default().show(context, |ui| {
                        ui.label("Содержимое анализа");
                    });
                },
            ));
        }
        output.expect("analysis footer should render twice")
    }

    #[test]
    fn analysis_footer_stays_at_bottom_and_remains_visible_at_different_widths() {
        let share_note = "Доли считаются отдельно для доходов и расходов и только внутри одной валюты. RUB и USD не пересчитываются.";
        let forecast_note = "Прогноз и оценка процентов не рассчитываются: методика и необходимые исходные данные не согласованы.";

        for width in [700.0, 1_280.0, 1_920.0] {
            let output = render_analysis_footer_at_width(ModuleTab::Analysis, width);
            let body = text_positions(&output, "Содержимое анализа")[0];
            let share = text_positions(&output, share_note)[0];
            let forecast = text_positions(&output, forecast_note)[0];

            assert!(
                share.y > body.y,
                "footer should follow the page content at width {width}"
            );
            assert!(
                share.y > 540.0,
                "footer should stay near the bottom at width {width}"
            );
            assert!(
                forecast.y > share.y,
                "footer notes should keep their order at width {width}"
            );
        }
    }

    #[test]
    fn analysis_footer_is_hidden_on_other_tabs() {
        let output = render_analysis_footer_at_width(ModuleTab::Income, 1_280.0);

        assert!(text_positions(&output, "Доли считаются отдельно для доходов и расходов и только внутри одной валюты. RUB и USD не пересчитываются.").is_empty());
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
    fn category_tree_labels_show_paths_and_keep_inactive_categories() {
        let mut categories = vec![
            category(3, Some(2), true),
            category(20, None, true),
            category(4, Some(3), false),
            category(2, None, false),
            category(5, Some(2), true),
            category(6, Some(20), true),
        ];
        categories[0].name = "Разное".to_owned();
        categories[1].name = "Транспорт".to_owned();
        categories[2].name = "История".to_owned();
        categories[3].name = "Расходы".to_owned();
        categories[4].name = "Разное".to_owned();
        categories[5].name = "Разное".to_owned();

        let rows = category_tree_labels(&categories);

        assert_eq!(
            rows,
            vec![
                (20, "Транспорт".to_owned()),
                (6, "Транспорт → Разное".to_owned()),
                (2, "Расходы".to_owned()),
                (3, "Расходы → Разное".to_owned()),
                (4, "Расходы → Разное → История".to_owned()),
                (5, "Расходы → Разное".to_owned()),
            ]
        );
    }

    #[test]
    fn analysis_refresh_loads_both_category_lists_including_inactive_categories() {
        let database =
            finance_db::Database::open_in_memory().expect("in-memory SQLite should open");
        database
            .execute_batch(include_str!("../../db/migrations/V1__initial.sql"))
            .expect("initial migration should apply");
        database
            .execute_batch(include_str!(
                "../../db/migrations/V2__transactions_and_directories.sql"
            ))
            .expect("transaction migration should apply");

        let income_parent = finance_db::save_category(
            &database,
            TransactionKind::Income,
            None,
            "Доходы",
            None,
            true,
        )
        .expect("income parent category should save");
        let income_child = finance_db::save_category(
            &database,
            TransactionKind::Income,
            None,
            "Премии",
            Some(income_parent),
            false,
        )
        .expect("inactive income child category should save");
        let expense_category = finance_db::save_category(
            &database,
            TransactionKind::Expense,
            None,
            "Покупки",
            None,
            true,
        )
        .expect("expense category should save");

        let mut app = FinanceApp::new(database);
        app.active_tab = ModuleTab::Analysis;
        app.refresh_all();

        assert_eq!(
            app.analysis_income_categories
                .iter()
                .map(|category| category.id)
                .collect::<Vec<_>>(),
            vec![income_parent, income_child]
        );
        assert_eq!(
            app.analysis_expense_categories
                .iter()
                .map(|category| category.id)
                .collect::<Vec<_>>(),
            vec![expense_category]
        );
        assert!(!app.analysis_income_categories[1].is_active);
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
    fn transaction_column_widths_fill_available_space_or_keep_minimum_widths() {
        let minimum_width: f32 = TRANSACTION_COLUMN_MIN_WIDTHS.iter().sum();

        for available_width in [1_200.0, 1_840.0, 700.0] {
            let widths = transaction_column_widths(available_width);
            let table_width: f32 = widths.iter().sum();
            let expected_width = available_width.max(minimum_width);

            assert!((table_width - expected_width).abs() < 0.01);
            assert!(widths
                .iter()
                .zip(TRANSACTION_COLUMN_MIN_WIDTHS)
                .all(|(width, minimum)| *width >= minimum));
        }
    }

    #[test]
    fn income_and_expense_tables_align_columns_expand_and_wrap_long_comments() {
        let long_comment = "Длинный комментарий для проверки переноса текста. ".repeat(12);

        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            let mut app = filtered_transaction_app(kind);
            app.transactions[0].comment = long_comment.clone();
            let mut second_transaction = app.transactions[0].clone();
            second_transaction.id += 100;
            second_transaction.comment = "Следующая запись".to_owned();
            app.transactions.push(second_transaction);

            for width in [1_280.0, 1_920.0, 700.0] {
                let context = egui::Context::default();
                let output = render_transactions_at_width(&mut app, &context, kind, width);
                let date_header = text_positions(&output, "Дата")[0];
                let date_cells = text_positions(&output, "15.01.2024");
                let amount_header = text_positions(&output, "Сумма")[0];
                let amount_cell = text_positions(&output, "20")[0];
                let category_header = text_positions(&output, "Категория")[0];
                let category_cell = *text_positions(&output, "Основная")
                    .last()
                    .expect("transaction category should be rendered");
                let comment_header = text_positions(&output, "Комментарий")[0];
                let comment_shape =
                    output
                        .shapes
                        .iter()
                        .find_map(|clipped_shape| match &clipped_shape.shape {
                            egui::epaint::Shape::Text(text_shape)
                                if text_shape.galley.job.text == long_comment =>
                            {
                                Some((text_shape.pos, text_shape.galley.size().y))
                            }
                            _ => None,
                        });
                let next_comment = text_positions(&output, "Следующая запись")[0];
                let action_positions = text_positions(&output, "Действия");

                assert_eq!(date_cells.len(), 2);
                assert!(
                    (date_header.x - date_cells[0].x).abs() < 0.1,
                    "date header and cell should align at width {width}: header={}, cell={}",
                    date_header.x,
                    date_cells[0].x
                );
                assert!((amount_header.x - amount_cell.x).abs() < 0.1);
                assert!((category_header.x - category_cell.x).abs() < 0.1);
                let (comment_position, comment_height) =
                    comment_shape.expect("long comment should be rendered as wrapped text");
                assert!((comment_header.x - comment_position.x).abs() < 0.1);
                assert!(comment_height > TABLE_ROW_HEIGHT);
                assert!(next_comment.y >= comment_position.y + comment_height);
                if width >= 1_280.0 {
                    let actions_header = action_positions[0];
                    assert!(actions_header.x > width * 0.75);
                    assert!(actions_header.x < width);
                } else {
                    assert!(action_positions.is_empty());
                }
            }
        }
    }

    #[test]
    fn income_and_expense_forms_keep_requested_field_and_footer_layout() {
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            for width in [1_280.0, 1_920.0] {
                for (amount, footer_text, has_division_warning) in [
                    ("10 + 20", "Итого: 30", false),
                    ("1 / 8", "Итого: 0.13", true),
                    ("10 +", "Некорректная сумма", false),
                    ("10 /", "Некорректная сумма", true),
                ] {
                    let mut app = filtered_transaction_app(kind);
                    app.transaction_form = super::TransactionForm {
                        date: "2024-01-15".to_owned(),
                        category_id: Some(1),
                        amount: amount.to_owned(),
                        currency: Some(Currency::Rub),
                        comment: "Тестовый комментарий".to_owned(),
                        ..Default::default()
                    };
                    let context = egui::Context::default();
                    let output = render_transaction_form_at_width(&mut app, &context, kind, width);

                    let date = text_positions(&output, "Дата")[0];
                    let amount_label = text_positions(&output, "Сумма")[0];
                    let currency = text_positions(&output, "Валюта")[0];
                    let category = text_positions(&output, "Категория")[0];
                    assert!(date.x < amount_label.x);
                    assert!(amount_label.x < currency.x);
                    assert!(currency.x < category.x);
                    assert!(amount_label.x - date.x > 140.0);
                    assert!(currency.x - amount_label.x > 240.0);
                    assert!(category.x - currency.x > 130.0);
                    for field in [amount_label, currency, category] {
                        assert!(
                            (field.y - date.y).abs() < 1.0,
                            "date, amount, currency, and category should share one row at width {width}"
                        );
                    }

                    let comment = text_positions(&output, "Комментарий")[0];
                    assert!(comment.y - date.y > 24.0);

                    let footer = text_positions(&output, footer_text)[0];
                    assert_eq!(
                        text_positions(&output, DIVISION_ROUNDING_WARNING).len(),
                        usize::from(has_division_warning),
                        "division warning visibility for amount {amount:?}"
                    );
                    let save = text_positions(&output, "Сохранить")[0];
                    let cancel = text_positions(&output, "Отмена")[0];
                    assert!(footer.y - comment.y > 56.0);
                    assert!(
                        (footer.y - save.y).abs() < 1.0,
                        "the total or amount error should share the buttons' row at width {width}"
                    );
                    assert!((save.y - cancel.y).abs() < 1.0);
                    assert!(footer.x < save.x);
                    assert!(save.x < cancel.x);
                    assert!(
                        cancel.x > width * 0.75,
                        "save and cancel buttons should be aligned on the right at width {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn savings_filters_render_in_search_bank_status_order_with_updated_actions() {
        let database =
            finance_db::Database::open_in_memory().expect("in-memory SQLite should open");
        let mut savings_page = SavingsPage::default();
        let context = egui::Context::default();
        let output = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                savings_page.render(context, ui, &database, &[]);
            });
        });

        let search = text_positions(&output, "Поиск")[0];
        let bank = text_positions(&output, "Банк")[0];
        let status = text_positions(&output, "Статус")[0];
        assert!(search.x < bank.x && bank.x < status.x);
        assert_eq!(text_positions(&output, "Применить").len(), 1);
        assert_eq!(text_positions(&output, "Сбросить фильтры").len(), 1);
        assert!(text_positions(&output, "Применить фильтры").is_empty());
        assert!(text_positions(&output, "Сбросить").is_empty());
    }

    #[test]
    fn currency_directory_adds_currency_edits_rate_and_keeps_primary_first() {
        let mut app = directory_app();
        app.directory_right_tab = DirectoryRightTab::Currencies;
        app.currency_form = CurrencyForm {
            code: "EUR".to_owned(),
            name: "Евро".to_owned(),
            ..CurrencyForm::default()
        };
        app.show_currency_form = true;
        app.save_currency();

        let euro = app
            .currencies
            .iter()
            .find(|currency| currency.code == "EUR")
            .cloned()
            .unwrap();
        app.edit_currency_rate(&euro);
        app.currency_form.rate = "95.125".to_owned();
        app.currency_form.rate_date = "2026-02-01".to_owned();
        app.save_currency();

        let euro = app
            .currencies
            .iter()
            .find(|currency| currency.code == "EUR")
            .unwrap();
        assert_eq!(
            euro.rate_to_primary.map(|rate| rate.to_string()),
            Some("95.125".to_owned())
        );
        assert_eq!(
            currency_rate_label(euro, "RUB"),
            "1 EUR = 95.125 RUB · курс от 01.02.2026"
        );

        let context = egui::Context::default();
        let output = render_directories_at_width(&mut app, &context, 1280.0);
        assert!(text_positions(&output, "Валюта и курс к основной").len() == 1);
        let rub_y = text_positions(&output, "RUB")[0].y;
        let usd_y = text_positions(&output, "USD")[0].y;
        assert!(rub_y < usd_y, "the primary currency should render first");

        app.change_primary_currency("EUR");
        assert_eq!(app.currencies[0].code, "EUR");
        assert!(app.currencies[0].is_primary);
    }

    #[test]
    fn currency_directory_marks_transaction_currency_as_used() {
        let mut app = directory_app();
        finance_db::add_currency(&app.database, "EUR", "Евро").unwrap();
        app.database
            .execute_batch(
                "INSERT INTO income_categories (name) VALUES ('Зарплата');
                 INSERT INTO incomes (date, category_id, amount, currency)
                    VALUES ('2026-01-01', 1, '12.50', 'EUR');",
            )
            .unwrap();

        app.refresh_directories();

        assert!(app.used_currencies.contains("EUR"));
        assert!(matches!(
            finance_db::delete_currency(&app.database, "EUR"),
            Err(finance_db::DbError::CurrencyInUse)
        ));
    }

    #[test]
    fn directory_scroll_areas_have_distinct_ids_when_rendered() {
        let mut app = directory_app();
        app.database
            .execute(
                "INSERT INTO income_categories (name) VALUES ('Тестовая категория')",
                [],
            )
            .expect("test category should be inserted");
        app.database
            .execute("INSERT INTO banks (name) VALUES ('Тестовый банк')", [])
            .expect("test bank should be inserted");
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
        assert_ne!(
            DIRECTORY_CATEGORIES_SCROLL_AREA_ID,
            DIRECTORY_BANKS_SCROLL_AREA_ID
        );
    }

    #[test]
    fn directory_name_columns_are_left_aligned_after_second_frame() {
        let mut app = directory_app();
        app.database
            .execute(
                "INSERT INTO income_categories (name) VALUES ('Корневая категория')",
                [],
            )
            .expect("root category should be inserted");
        app.database
            .execute(
                "INSERT INTO income_categories (name, parent_id) VALUES ('Дочерняя категория', 1)",
                [],
            )
            .expect("child category should be inserted");
        app.database
            .execute(
                "INSERT INTO banks (name) VALUES ('Первый банк'), ('Второй банк')",
                [],
            )
            .expect("test banks should be inserted");
        app.refresh_directories();

        let context = egui::Context::default();
        let narrow_output = render_directories_at_width(&mut app, &context, 1280.0);
        let wide_output = render_directories_at_width(&mut app, &context, 1600.0);

        for output in [&narrow_output, &wide_output] {
            let category_header_x = text_positions(output, "Категория")[0].x;
            for text in ["Корневая категория", "　↳ Дочерняя категория"]
            {
                let text_start_x = text_positions(output, text)[0].x;
                assert!(
                    (text_start_x - category_header_x).abs() < 0.5,
                    "category text should start at the header's left edge: {text} ({text_start_x} vs {category_header_x})"
                );
            }

            let bank_header_x = text_positions(output, "Банк")[0].x;
            for text in ["Первый банк", "Второй банк"] {
                let text_start_x = text_positions(output, text)[0].x;
                assert!(
                    (text_start_x - bank_header_x).abs() < 0.5,
                    "bank text should start at the header's left edge: {text} ({text_start_x} vs {bank_header_x})"
                );
            }
        }

        let action_column_xs = |output: &egui::FullOutput| {
            let mut xs: Vec<_> = text_positions(output, "Изменить")
                .into_iter()
                .map(|position| position.x)
                .collect();
            xs.sort_by(f32::total_cmp);
            xs.dedup_by(|left, right| (*left - *right).abs() < 0.5);
            xs
        };
        let narrow_action_xs = action_column_xs(&narrow_output);
        let wide_action_xs = action_column_xs(&wide_output);
        assert_eq!(
            narrow_action_xs.len(),
            2,
            "both action columns should render"
        );
        assert_eq!(wide_action_xs.len(), 2, "both action columns should render");
        let panel_expansion = (1600.0_f32 - 1280.0) / 2.0;
        for (index, (narrow_x, wide_x)) in narrow_action_xs.iter().zip(&wide_action_xs).enumerate()
        {
            let expected_expansion = panel_expansion * (index as f32 + 1.0);
            assert!(
                (wide_x - narrow_x - expected_expansion).abs() < 1.0,
                "the action column should move with the expanded name column: {narrow_x} to {wide_x}"
            );
        }
    }

    #[test]
    fn transaction_filters_can_be_cleared_individually_for_income_and_expenses() {
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            let mut app = filtered_transaction_app(kind);
            app.clear_search_filter(kind);
            assert!(app.search.is_empty());
            assert_eq!(app.from_date, "2024-01-01");
            assert_eq!(app.to_date, "2024-01-31");
            assert_eq!(app.category_filter, Some(1));
            assert_eq!(
                transaction_rows(&app),
                vec![
                    ("2024-01-15".to_owned(), 1, "пропустить".to_owned()),
                    ("2024-01-15".to_owned(), 1, "найти".to_owned()),
                ]
            );

            let mut app = filtered_transaction_app(kind);
            app.clear_from_date_filter(kind);
            assert_eq!(app.search, "найти");
            assert!(app.from_date.is_empty());
            assert_eq!(app.to_date, "2024-01-31");
            assert_eq!(app.category_filter, Some(1));
            assert_eq!(
                transaction_rows(&app),
                vec![
                    ("2024-01-15".to_owned(), 1, "найти".to_owned()),
                    ("2023-12-15".to_owned(), 1, "найти".to_owned()),
                ]
            );

            let mut app = filtered_transaction_app(kind);
            app.clear_to_date_filter(kind);
            assert_eq!(app.search, "найти");
            assert_eq!(app.from_date, "2024-01-01");
            assert!(app.to_date.is_empty());
            assert_eq!(app.category_filter, Some(1));
            assert_eq!(
                transaction_rows(&app),
                vec![
                    ("2024-02-15".to_owned(), 1, "найти".to_owned()),
                    ("2024-01-15".to_owned(), 1, "найти".to_owned()),
                ]
            );

            let mut app = filtered_transaction_app(kind);
            app.clear_category_filter(kind);
            assert_eq!(app.search, "найти");
            assert_eq!(app.from_date, "2024-01-01");
            assert_eq!(app.to_date, "2024-01-31");
            assert_eq!(app.category_filter, None);
            assert_eq!(
                transaction_rows(&app),
                vec![
                    ("2024-01-15".to_owned(), 2, "найти".to_owned()),
                    ("2024-01-15".to_owned(), 1, "найти".to_owned()),
                ]
            );
        }
    }

    #[test]
    fn all_transaction_filters_can_be_reset_for_income_and_expenses() {
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            let mut app = filtered_transaction_app(kind);
            app.reset_transaction_filters(kind);

            assert!(app.search.is_empty());
            assert!(app.from_date.is_empty());
            assert!(app.to_date.is_empty());
            assert_eq!(app.category_filter, None);
            assert_eq!(app.transactions.len(), 5);
        }
    }
}
