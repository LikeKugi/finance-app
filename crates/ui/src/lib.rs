use std::collections::HashSet;
use std::path::PathBuf;

use chrono::NaiveDate;
use eframe::egui;
use finance_core::{
    Bank, Category, Currency, CurrencyDefinition, FinancialTransaction, PaymentDocument,
    TransactionBalance, TransactionKind,
};
use finance_db::{self, Database};

mod analysis_page;
mod category_tree;
mod common;
mod date_input;
mod directories_page;
mod expense_page;
mod income_page;
mod payment_documents_page;
mod savings_page;
mod transactions_page;

use analysis_page::AnalysisPage;
use common::{
    render_analysis_footer, render_transaction_balance_footer, render_transaction_list_total,
};
use date_input::DateInputState;
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
            Self::PaymentDocuments => "Платёжные карты",
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
    currency_filter: Option<Currency>,
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
            currency_filter: None,
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
        self.refresh_currencies();
        match finance_db::list_used_currency_codes(&self.database) {
            Ok(currencies) => self.used_currencies = currencies.into_iter().collect(),
            Err(error) => self.show_error(error.to_string()),
        }
    }

    fn refresh_currencies(&mut self) -> bool {
        match finance_db::list_currencies(&self.database) {
            Ok(currencies) => {
                self.currencies = currencies;
                true
            }
            Err(error) => {
                self.currencies.clear();
                self.show_error(error.to_string());
                false
            }
        }
    }

    fn primary_currency_code(&self) -> Option<&str> {
        self.currencies
            .iter()
            .find(|currency| currency.is_primary)
            .map(|currency| currency.code.as_str())
    }

    fn show_error(&mut self, message: String) {
        self.message = Some((message, true));
    }

    fn show_success(&mut self, message: &str) {
        self.message = Some((message.to_owned(), false));
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
                if render_transaction_list_total(
                    ui,
                    &self.transactions,
                    self.primary_currency_code(),
                ) {
                    self.show_error("Переполнение при подсчёте итоговой суммы".to_owned());
                }
                render_transaction_balance_footer(
                    ui,
                    self.transaction_balances.as_ref(),
                    self.transaction_balance_period,
                    self.primary_currency_code(),
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
                ModuleTab::Income => income_page::render(self, ui),
                ModuleTab::Expenses => expense_page::render(self, ui),
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
    use finance_core::{Currency, FinancialTransaction, TransactionBalance};

    use super::category_tree::{category_tree_labels, category_tree_rows};
    use super::common::{
        render_analysis_footer, render_transaction_balance_footer, render_transaction_list_total,
        TABLE_ROW_HEIGHT,
    };
    use super::directories_page::currency_rate_label;
    use super::transactions_page::{
        transaction_column_widths, DIVISION_ROUNDING_WARNING, TRANSACTION_COLUMN_MIN_WIDTHS,
    };
    use super::{
        AnalysisPage, Category, CurrencyForm, DirectoryRightTab, FinanceApp, ModuleTab,
        SavingsPage, TransactionKind, DIRECTORY_BANKS_SCROLL_AREA_ID,
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
        database
            .execute_batch(include_str!("../../db/migrations/V8__currencies.sql"))
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
                finance_db::list_transactions(&app.database, kind, None, None, None, None, "")
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
                finance_db::list_transactions(&app.database, kind, None, None, None, None, "")
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
    fn transaction_refresh_loads_primary_currency_on_income_and_expenses_pages() {
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            let app = filtered_transaction_app(kind);

            assert_eq!(app.primary_currency_code(), Some("RUB"));
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
        for active_tab in [ModuleTab::Income, ModuleTab::Expenses] {
            for (primary_currency, expected, other_currency) in [
                (
                    "RUB",
                    "Доходы за весь период: 100 RUB − Расходы за весь период: 25 RUB = Сальдо за весь период: 75 RUB",
                    "USD",
                ),
                (
                    "USD",
                    "Доходы за весь период: 20 USD − Расходы за весь период: 5 USD = Сальдо за весь период: 15 USD",
                    "RUB",
                ),
            ] {
                let rendered_text =
                    transaction_balance_footer_texts(active_tab, Some(primary_currency));
                assert_eq!(
                    rendered_text.iter().filter(|text| text.as_str() == expected).count(),
                    1,
                    "rendered labels: {rendered_text:?}"
                );
                assert!(
                    rendered_text.iter().all(|text| !text.contains(other_currency)),
                    "the footer should show only {primary_currency}: {rendered_text:?}"
                );
                assert!(
                    rendered_text
                        .iter()
                        .any(|text| text.contains("Сальдо операций не включает")),
                    "rendered labels: {rendered_text:?}"
                );
            }
        }
    }

    #[test]
    fn transaction_balance_footer_explains_unsupported_or_missing_primary_currency() {
        let unsupported = transaction_balance_footer_texts(ModuleTab::Income, Some("EUR"));
        assert!(unsupported.iter().any(|text| {
            text.contains("основная валюта EUR не поддерживается")
        }));
        assert!(unsupported
            .iter()
            .all(|text| !text.contains("Доходы за весь период:")));

        let missing = transaction_balance_footer_texts(ModuleTab::Expenses, None);
        assert!(missing
            .iter()
            .any(|text| text.contains("не удалось определить основную валюту")));
        assert!(missing
            .iter()
            .all(|text| !text.contains("Доходы за весь период:")));
    }

    #[test]
    fn transaction_list_total_shows_only_the_primary_currency() {
        let transactions = [
            transaction(1, Currency::Rub, 100),
            transaction(2, Currency::Usd, 20),
        ];
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
                        assert!(!render_transaction_list_total(
                            ui,
                            &transactions,
                            Some("USD")
                        ));
                    });
                },
            ));
        }
        let output = output.expect("transaction total should render twice");
        assert_eq!(
            text_positions(&output, "Записей: 2   Итого: 20 USD").len(),
            1
        );
        assert_eq!(text_positions(&output, "100 RUB").len(), 0);
    }

    fn transaction_balance_footer_texts(
        active_tab: ModuleTab,
        primary_currency: Option<&str>,
    ) -> Vec<String> {
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
                                    primary_currency,
                                );
                            },
                        );
                    }
                },
            ));
        }
        output
            .expect("balance footer should render twice")
            .shapes
            .iter()
            .filter_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) => Some(text_shape.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn transaction(id: i64, currency: Currency, amount: i64) -> FinancialTransaction {
        FinancialTransaction {
            id,
            date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            category_id: id,
            category_name: "Категория".to_owned(),
            amount: amount.into(),
            currency,
            comment: String::new(),
            payment_document_id: None,
            affects_deposit_rate: false,
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
            app.currency_filter = Some(Currency::Usd);
            app.reset_transaction_filters(kind);

            assert!(app.search.is_empty());
            assert!(app.from_date.is_empty());
            assert!(app.to_date.is_empty());
            assert_eq!(app.category_filter, None);
            assert_eq!(app.currency_filter, None);
            assert_eq!(app.transactions.len(), 5);
        }
    }

    #[test]
    fn transaction_currency_filter_applies_and_clears_for_income_and_expenses() {
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            let mut app = filtered_transaction_app(kind);
            finance_db::save_transaction(
                &app.database,
                kind,
                None,
                NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
                1,
                1.into(),
                Currency::Usd,
                "найти",
            )
            .unwrap();
            app.refresh_transactions(kind);
            assert_eq!(app.transactions.len(), 2);
            let balances = app.transaction_balances;

            app.currency_filter = Some(Currency::Rub);
            app.refresh_transactions(kind);
            assert_eq!(app.transactions.len(), 1);
            assert_eq!(app.transactions[0].currency, Currency::Rub);
            assert_eq!(app.transaction_balances, balances);

            app.currency_filter = Some(Currency::Usd);
            app.refresh_transactions(kind);
            assert_eq!(app.transactions.len(), 1);
            assert_eq!(app.transactions[0].currency, Currency::Usd);
            assert_eq!(app.transaction_balances, balances);

            app.clear_currency_filter(kind);
            assert_eq!(app.currency_filter, None);
            assert_eq!(app.search, "найти");
            assert_eq!(app.from_date, "2024-01-01");
            assert_eq!(app.to_date, "2024-01-31");
            assert_eq!(app.category_filter, Some(1));
            assert_eq!(app.transactions.len(), 2);
            assert_eq!(app.transaction_balances, balances);
        }
    }
}
