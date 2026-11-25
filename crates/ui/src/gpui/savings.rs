//! Страница «Накопления» на `gpui-kit`. Поведение соответствует
//! `savings_page.rs`; SQL и финансовые правила остаются в `finance-db` и
//! `finance-core`.

use std::cmp::Ordering;
use std::rc::Rc;

use chrono::NaiveDate;
use finance_core::{
    calculate_interest_amount, canonical_amount, parse_amount, parse_non_negative_decimal, Bank,
    Currency, InterestDestination, PaymentCardType, PaymentDocument, SavingsAccount,
    SavingsDetails, SavingsTerm, TransactionKind,
};
use finance_db::{Database, SavingsInterestIncome, SavingsInterestIncomeRecipient};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::table::{
    Column, ColumnFixed, ColumnGroup, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::tooltip::Tooltip as ComponentTooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::StatefulInteractiveElement;
use gpui_kit::*;
use rust_decimal::Decimal;

use super::helpers::{card_label, format_currency, format_date, DATE_FORMAT};
use super::selectable::{
    selectable_message, selectable_right_aligned_table_cell, selectable_table_cell, selectable_text,
};
use super::support::{category_tree_labels, eligible_cards};

const ALL_LABEL: &str = "Все";
const ALL_BANKS_LABEL: &str = "Все банки";
const NO_CURRENCY_LABEL: &str = "Не указана";
const TERMS: [SavingsTerm; 2] = [SavingsTerm::Limited, SavingsTerm::Unlimited];
const COLUMN_HEADINGS: [&str; 13] = [
    "Название",
    "Банк",
    "Сумма",
    "Валюта",
    "Базовая",
    "Повышенная",
    "Условие",
    "Дата открытия",
    "Срок",
    "Дата",
    "Начисление",
    "",
    "Действия",
];
const RATE_GROUP_LABEL: &str = "Ставка";
const RATE_GROUP_SPAN: usize = 3;
const INTEREST_GROUP_LABEL: &str = "Выплата процентов";
const INTEREST_GROUP_SPAN: usize = 3;
const DEFAULT_COLUMN_WIDTHS: [f32; 13] = [
    120.0, 85.0, 90.0, 70.0, 80.0, 170.0, 90.0, 95.0, 115.0, 105.0, 120.0, 40.0, 160.0,
];
const ACTIONS_COLUMN_WIDTH: f32 = 160.0;
const INTEREST_ACTION_COLUMN_WIDTH: f32 = 40.0;
const MIN_COLUMN_WIDTH: f32 = 60.0;
const MAX_COLUMN_WIDTH: f32 = 1200.0;
const SAVINGS_COLUMN_WIDTHS_KEY: &str = "table_columns.savings.v4";
const PREVIOUS_SAVINGS_COLUMN_WIDTHS_KEY: &str = "table_columns.savings.v3";
const OLDER_SAVINGS_COLUMN_WIDTHS_KEY: &str = "table_columns.savings.v2";
const LEGACY_SAVINGS_COLUMN_WIDTHS_KEY: &str = "table_columns.savings";
const COL_TITLE: usize = 0;
const COL_BANK: usize = 1;
const COL_AMOUNT: usize = 2;
const COL_BASE_RATE: usize = 4;
const COL_RATE_AFTER_THRESHOLD: usize = 5;
const COL_THRESHOLD: usize = 6;
const COL_OPENED_DATE: usize = 7;
const COL_TERM: usize = 8;
const COL_INTEREST_DATE: usize = 9;
const COL_INTEREST_ACTION: usize = 11;
const COL_ACTIONS: usize = 12;
const FOOTER_NOTE: &str = "Даты открытия и начисления процентов сохраняются как справочные значения; процентный доход здесь не рассчитывается без истории остатка и правил начисления банка. Повышенная ставка показана как условие, а не как подтверждённая доходность.";
const FOOTER_WARNING: &str =
    "Валюта накопления указывается отдельно; фактические начисления процентов здесь не рассчитываются.";

/// Группы объединяют ставки и параметры начисления процентов.
/// Пустые группы сохраняют позиции остальных колонок, включая закреплённую «Действия».
fn savings_group_headers() -> Vec<Vec<ColumnGroup>> {
    vec![vec![
        ColumnGroup::new("", COL_BASE_RATE),
        ColumnGroup::new(RATE_GROUP_LABEL, RATE_GROUP_SPAN),
        ColumnGroup::new("", COL_INTEREST_DATE - COL_THRESHOLD - 1),
        ColumnGroup::new(INTEREST_GROUP_LABEL, INTEREST_GROUP_SPAN),
        ColumnGroup::new(
            "",
            COLUMN_HEADINGS.len() - COL_INTEREST_DATE - INTEREST_GROUP_SPAN,
        ),
    ]]
}

const BANK_FILTER_ITEM_ID: &str = "savings-bank-filter-item";
const TERM_FILTER_ITEM_ID: &str = "savings-term-filter-item";
const DESTINATION_FILTER_ITEM_ID: &str = "savings-destination-filter-item";

type Choice = Entity<SelectState<Vec<SharedString>>>;
type TooltipChoice = Entity<SelectState<Vec<TooltipFilterItem>>>;

#[derive(Clone)]
struct TooltipFilterItem {
    id: usize,
    label: String,
    item_id: &'static str,
}

impl SelectItem for TooltipFilterItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.label
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip = self.label.clone();
        let item_id = self.item_id;
        div()
            .id((item_id, self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}

fn tooltip_items(labels: Vec<String>, item_id: &'static str) -> Vec<TooltipFilterItem> {
    labels
        .into_iter()
        .enumerate()
        .map(|(id, label)| TooltipFilterItem { id, label, item_id })
        .collect()
}

fn tooltip_choice(
    labels: Vec<String>,
    selected: Option<usize>,
    item_id: &'static str,
    window: &mut Window,
    cx: &mut Context<SavingsPage>,
) -> TooltipChoice {
    let items = tooltip_items(labels, item_id);
    cx.new(|cx| {
        SelectState::new(
            items,
            selected.map(|row| IndexPath::default().row(row)),
            window,
            cx,
        )
    })
}

fn choice(
    items: Vec<String>,
    selected: Option<usize>,
    window: &mut Window,
    cx: &mut Context<SavingsPage>,
) -> Choice {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| {
        SelectState::new(
            items,
            selected.map(|row| IndexPath::default().row(row)),
            window,
            cx,
        )
    })
}

fn selected_choice_row(state: &Entity<SelectState<Vec<SharedString>>>, cx: &App) -> Option<usize> {
    state.read(cx).selected_index(cx).map(|index| index.row)
}

fn currency_labels(first: &str) -> Vec<String> {
    std::iter::once(first.to_owned())
        .chain(Currency::ALL.into_iter().map(|c| c.code().to_owned()))
        .collect()
}

fn currency_by_code(label: &str) -> Option<Currency> {
    Currency::ALL.into_iter().find(|c| c.code() == label)
}

fn status_labels() -> Vec<String> {
    vec![
        ALL_LABEL.to_owned(),
        "Активные".to_owned(),
        "Неактивные".to_owned(),
    ]
}

fn term_filter_labels() -> Vec<String> {
    std::iter::once(ALL_LABEL.to_owned())
        .chain(TERMS.into_iter().map(|term| term.title().to_owned()))
        .collect()
}

fn destination_filter_labels() -> Vec<String> {
    std::iter::once(ALL_LABEL.to_owned())
        .chain(
            InterestDestination::ALL
                .into_iter()
                .map(|destination| destination.title().to_owned()),
        )
        .collect()
}

fn filter_savings(
    entries: Vec<SavingsAccount>,
    term: Option<SavingsTerm>,
    destination: Option<InterestDestination>,
) -> Vec<SavingsAccount> {
    entries
        .into_iter()
        .filter(|entry| {
            term.is_none_or(|term| entry.term_type == term)
                && destination.is_none_or(|destination| entry.interest_destination == destination)
        })
        .collect()
}

/// Значения полей формы в виде текста и выбранных вариантов; не зависит от GUI.
#[derive(Clone, Default, PartialEq)]
struct SavingsForm {
    id: Option<i64>,
    bank_id: Option<i64>,
    currency: Option<Currency>,
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
}

impl SavingsForm {
    fn new_for(banks: &[Bank]) -> Self {
        Self {
            bank_id: banks.first().map(|bank| bank.id),
            base_rate: "0".to_owned(),
            term_type: Some(SavingsTerm::Unlimited),
            interest_destination: Some(InterestDestination::Reinvest),
            is_active: true,
            ..Self::default()
        }
    }

    fn from_account(account: &SavingsAccount) -> Self {
        let date_text = |date: Option<NaiveDate>| {
            date.map_or_else(String::new, |date| date.format(DATE_FORMAT).to_string())
        };
        Self {
            id: Some(account.id),
            bank_id: Some(account.bank_id),
            currency: account.currency,
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
            opened_date: date_text(account.opened_date),
            interest_accrual_date: date_text(account.interest_accrual_date),
            term_type: Some(account.term_type),
            term_end_date: date_text(account.term_end_date),
            interest_destination: Some(account.interest_destination),
            is_active: account.is_active,
        }
    }

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
                NaiveDate::parse_from_str(self.term_end_date.trim(), DATE_FORMAT)
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
            currency: self.currency,
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
    NaiveDate::parse_from_str(value.trim(), DATE_FORMAT)
        .map(Some)
        .map_err(|_| format!("Введите дату {field_name} в формате ГГГГ-ММ-ДД"))
}

#[derive(Debug, Clone, PartialEq)]
struct InterestAccrualInput {
    accrual_date: Option<NaiveDate>,
    interest_destination: Option<InterestDestination>,
    create_income: bool,
    income_date: Option<NaiveDate>,
    amount: String,
    currency: Option<Currency>,
    category_id: Option<i64>,
    recipient: Option<SavingsInterestIncomeRecipient>,
    comment: String,
    higher_rate_applied: bool,
    manual_amount: bool,
}

fn selected_date(state: &DatePickerState) -> Option<NaiveDate> {
    match state.date() {
        Date::Single(date) => date,
        Date::Range(_, _) => None,
    }
}

fn default_interest_amount(
    account: &SavingsAccount,
    higher_rate_applied: bool,
) -> Result<Decimal, finance_core::DomainError> {
    let rate = if higher_rate_applied {
        account
            .rate_after_threshold
            .ok_or(finance_core::DomainError::InvalidSavingsRate)?
    } else {
        account.base_rate
    };
    calculate_interest_amount(account.amount, rate)
}

fn interest_amount_for_rate_change(
    account: &SavingsAccount,
    higher_rate_applied: bool,
    manual_amount: bool,
) -> Result<Option<Decimal>, finance_core::DomainError> {
    if manual_amount {
        Ok(None)
    } else {
        default_interest_amount(account, higher_rate_applied).map(Some)
    }
}

fn build_interest_accrual_update(
    input: &InterestAccrualInput,
    account: &SavingsAccount,
) -> Result<
    (
        Option<NaiveDate>,
        InterestDestination,
        Option<SavingsInterestIncome>,
    ),
    String,
> {
    let interest_destination = input
        .interest_destination
        .ok_or_else(|| "Выберите место начисления процентов".to_owned())?;
    let income = if input.create_income {
        let date = input
            .income_date
            .ok_or_else(|| "Выберите дату дохода".to_owned())?;
        let category_id = input
            .category_id
            .ok_or_else(|| "Выберите категорию дохода".to_owned())?;
        let amount = parse_amount(&input.amount).map_err(|error| error.to_string())?;
        let currency = input
            .currency
            .ok_or_else(|| "Выберите валюту дохода".to_owned())?;
        let recipient = input
            .recipient
            .ok_or_else(|| "Выберите получателя дохода".to_owned())?;
        if recipient == SavingsInterestIncomeRecipient::Savings {
            if !account.is_active {
                return Err("Нельзя зачислить доход на неактивное накопление".to_owned());
            }
            match account.currency {
                None => {
                    return Err(
                        "Для зачисления на накопление сначала укажите его валюту в настройках"
                            .to_owned(),
                    );
                }
                Some(account_currency) if account_currency != currency => {
                    return Err("Валюта накопления должна совпадать с валютой дохода".to_owned());
                }
                Some(_) => {}
            }
        }
        Some(SavingsInterestIncome {
            date,
            category_id,
            amount,
            currency,
            comment: input.comment.clone(),
            recipient,
        })
    } else {
        None
    };

    Ok((input.accrual_date, interest_destination, income))
}

fn interest_recipient_choices(
    account: &SavingsAccount,
    cards: &[PaymentDocument],
    currency: Option<Currency>,
) -> (Vec<SavingsInterestIncomeRecipient>, Vec<String>) {
    let eligible = eligible_cards(cards, currency);
    let mut recipients = vec![SavingsInterestIncomeRecipient::WithoutCard];
    let mut labels = vec!["Без карты".to_owned()];
    for card in eligible {
        recipients.push(SavingsInterestIncomeRecipient::PaymentDocument(card.id));
        labels.push(card_label(card));
    }
    recipients.push(SavingsInterestIncomeRecipient::Savings);
    labels.push(format!("Текущее накопление: {}", account.title));
    (recipients, labels)
}

/// Состояние открытой формы: поля ввода и выбор вариантов.
struct Form {
    id: Option<i64>,
    title: Entity<InputState>,
    amount: Entity<InputState>,
    base_rate: Entity<InputState>,
    threshold_amount: Entity<InputState>,
    rate_after_threshold: Entity<InputState>,
    opened_date: Entity<DatePickerState>,
    interest_accrual_date: Entity<DatePickerState>,
    term_end_date: Entity<DatePickerState>,
    bank_select: Choice,
    currency_select: Choice,
    term_select: Choice,
    destination_select: Choice,
    bank_id: Option<i64>,
    currency: Option<Currency>,
    term_type: Option<SavingsTerm>,
    interest_destination: Option<InterestDestination>,
    use_threshold: bool,
    is_active: bool,
    snapshot: SavingsForm,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

struct InterestAccrualForm {
    account: SavingsAccount,
    accrual_date: Entity<DatePickerState>,
    interest_destination: InterestDestination,
    interest_destination_select: Choice,
    create_income: bool,
    income_date: Entity<DatePickerState>,
    income_amount: Entity<InputState>,
    income_currency: Option<Currency>,
    income_currency_select: Choice,
    category_ids: Vec<i64>,
    category_id: Option<i64>,
    category_select: Choice,
    cards: Vec<finance_core::PaymentDocument>,
    recipient_options: Vec<SavingsInterestIncomeRecipient>,
    recipient: Option<SavingsInterestIncomeRecipient>,
    recipient_select: Choice,
    comment: Entity<TextareaState>,
    higher_rate_applied: bool,
    manual_amount: bool,
    automatic_amount: String,
    error: Option<String>,
    snapshot: InterestAccrualInput,
    confirm_discard: bool,
    _subscriptions: Vec<Subscription>,
}

impl InterestAccrualForm {
    fn input(&self, cx: &App) -> InterestAccrualInput {
        InterestAccrualInput {
            accrual_date: selected_date(self.accrual_date.read(cx)),
            interest_destination: Some(self.interest_destination),
            create_income: self.create_income,
            income_date: selected_date(self.income_date.read(cx)),
            amount: self.income_amount.read(cx).value().to_string(),
            currency: self.income_currency,
            category_id: self.category_id,
            recipient: self.recipient,
            comment: self.comment.read(cx).value().to_string(),
            higher_rate_applied: self.higher_rate_applied,
            manual_amount: self.manual_amount,
        }
    }
}

pub(crate) struct SavingsPage {
    database: Rc<Database>,
    banks: Vec<Bank>,
    entries: Vec<SavingsAccount>,
    table: Entity<TableState<SavingsTable>>,
    message: Option<(String, bool)>,
    search: Entity<InputState>,
    bank_filter: Option<i64>,
    bank_filter_select: TooltipChoice,
    currency_filter: Option<Currency>,
    currency_filter_select: Choice,
    active_filter: Option<bool>,
    active_filter_select: Choice,
    term_filter: Option<SavingsTerm>,
    term_filter_select: TooltipChoice,
    destination_filter: Option<InterestDestination>,
    destination_filter_select: TooltipChoice,
    form: Option<Form>,
    interest_form: Option<InterestAccrualForm>,
    confirm_discard: bool,
    _subscriptions: Vec<Subscription>,
}

impl SavingsPage {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Название или банк"));
        let bank_filter_select = tooltip_choice(
            vec![ALL_BANKS_LABEL.to_owned()],
            Some(0),
            BANK_FILTER_ITEM_ID,
            window,
            cx,
        );
        let currency_filter_select = choice(currency_labels(ALL_LABEL), Some(0), window, cx);
        let active_filter_select = choice(status_labels(), Some(0), window, cx);
        let term_filter_select = tooltip_choice(
            term_filter_labels(),
            Some(0),
            TERM_FILTER_ITEM_ID,
            window,
            cx,
        );
        let destination_filter_select = tooltip_choice(
            destination_filter_labels(),
            Some(0),
            DESTINATION_FILTER_ITEM_ID,
            window,
            cx,
        );

        let mut subscriptions = vec![
            cx.subscribe_in(
                &search,
                window,
                |this: &mut Self, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.apply_filters(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &bank_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<TooltipFilterItem>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.bank_filter = value.as_deref().and_then(|label| {
                        this.banks
                            .iter()
                            .find(|bank| bank.name == label)
                            .map(|bank| bank.id)
                    });
                    this.apply_filters(cx);
                },
            ),
            cx.subscribe_in(
                &currency_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.currency_filter =
                        value.as_ref().and_then(|l| currency_by_code(l.as_ref()));
                    this.apply_filters(cx);
                },
            ),
            cx.subscribe_in(
                &active_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.active_filter = match value.as_ref().map(|l| l.as_ref()) {
                        Some("Активные") => Some(true),
                        Some("Неактивные") => Some(false),
                        _ => None,
                    };
                    this.apply_filters(cx);
                },
            ),
            cx.subscribe_in(
                &term_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<TooltipFilterItem>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.term_filter = value
                        .as_deref()
                        .and_then(|label| TERMS.into_iter().find(|term| term.title() == label));
                    this.apply_filters(cx);
                },
            ),
            cx.subscribe_in(
                &destination_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<TooltipFilterItem>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.destination_filter = value.as_deref().and_then(|label| {
                        InterestDestination::ALL
                            .into_iter()
                            .find(|destination| destination.title() == label)
                    });
                    this.apply_filters(cx);
                },
            ),
        ];
        subscriptions.shrink_to_fit();

        let widths = load_column_widths(&database);
        let table_delegate = SavingsTable::new(cx.entity().downgrade(), widths);
        let table = cx.new(|cx| TableState::new(table_delegate, window, cx).col_movable(false));
        subscriptions.push(cx.subscribe_in(
            &table,
            window,
            |this: &mut Self, _, event: &TableEvent, window, cx| match event {
                TableEvent::ColumnWidthsChanged(widths) => this.save_column_widths(widths, cx),
                TableEvent::DoubleClickedRow(row_ix) => {
                    let Some(id) = this
                        .table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*row_ix)
                        .map(|account| account.id)
                    else {
                        return;
                    };
                    this.begin_edit(id, window, cx);
                }
                _ => {}
            },
        ));

        let mut page = Self {
            database,
            banks: Vec::new(),
            entries: Vec::new(),
            table,
            message: None,
            search,
            bank_filter: None,
            bank_filter_select,
            currency_filter: None,
            currency_filter_select,
            active_filter: None,
            active_filter_select,
            term_filter: None,
            term_filter_select,
            destination_filter: None,
            destination_filter_select,
            form: None,
            interest_form: None,
            confirm_discard: false,
            _subscriptions: subscriptions,
        };
        page.reload_banks(window, cx);
        page.refresh(cx);
        page
    }

    /// Обновляет банки и список после изменений в справочниках.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_banks(window, cx);
        self.refresh(cx);
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some((message.into(), true));
        cx.notify();
    }

    fn save_column_widths(&mut self, widths: &[Pixels], cx: &mut Context<Self>) {
        let widths = column_widths_with_fixed_actions(
            widths.iter().map(|width| f32::from(*width)).collect(),
        );
        self.table
            .update(cx, |table, _| table.delegate_mut().set_widths(&widths));
        if let Err(error) = finance_db::set_app_setting(
            &self.database,
            SAVINGS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&widths),
        ) {
            self.error(format!("Не удалось сохранить ширину колонок: {error}"), cx);
        }
    }

    fn reload_banks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match finance_db::list_banks(&self.database) {
            Ok(banks) => self.banks = banks,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        }
        if let Some(id) = self.bank_filter {
            if !self.banks.iter().any(|bank| bank.id == id) {
                self.bank_filter = None;
            }
        }
        let labels = std::iter::once(ALL_BANKS_LABEL.to_owned())
            .chain(self.banks.iter().map(|bank| bank.name.clone()))
            .collect();
        let items = tooltip_items(labels, BANK_FILTER_ITEM_ID);
        let selected = self
            .bank_filter
            .and_then(|id| self.banks.iter().position(|bank| bank.id == id))
            .map_or(0, |position| position + 1);
        self.bank_filter_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(Some(IndexPath::default().row(selected)), window, cx);
        });
        self.rebuild_form_bank_select(window, cx);
    }

    fn rebuild_form_bank_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let items: Vec<SharedString> = self
            .banks
            .iter()
            .map(|bank| SharedString::from(bank.name.clone()))
            .collect();
        let selected = form
            .bank_id
            .and_then(|id| self.banks.iter().position(|bank| bank.id == id));
        form.bank_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(
                selected.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            );
        });
    }

    /// Загружает список по текущим фильтрам; ошибки показываются, а не маскируются.
    fn refresh(&mut self, cx: &mut Context<Self>) -> bool {
        let search = self.search.read(cx).value().to_string();
        match finance_db::list_savings_filtered(
            &self.database,
            self.bank_filter,
            self.active_filter,
            self.currency_filter,
            &search,
        ) {
            Ok(entries) => {
                let entries = filter_savings(entries, self.term_filter, self.destination_filter);
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_rows(entries.clone());
                    cx.notify();
                });
                self.entries = entries;
                cx.notify();
                true
            }
            Err(error) => {
                self.error(error.to_string(), cx);
                false
            }
        }
    }

    fn apply_filters(&mut self, cx: &mut Context<Self>) {
        if self.refresh(cx) {
            self.message = None;
        }
    }

    fn has_active_filters(&self, cx: &App) -> bool {
        !self.search.read(cx).value().is_empty()
            || self.bank_filter.is_some()
            || self.currency_filter.is_some()
            || self.active_filter.is_some()
            || self.term_filter.is_some()
            || self.destination_filter.is_some()
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.bank_filter = None;
        self.currency_filter = None;
        self.active_filter = None;
        self.term_filter = None;
        self.destination_filter = None;
        for select in [&self.currency_filter_select, &self.active_filter_select] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
            });
        }
        for select in [
            &self.bank_filter_select,
            &self.term_filter_select,
            &self.destination_filter_select,
        ] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
            });
        }
        self.apply_filters(cx);
    }

    // ---- форма ----

    fn begin_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let initial = SavingsForm::new_for(&self.banks);
        self.open_form(initial, window, cx);
    }

    fn begin_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.entries.iter().find(|account| account.id == id) else {
            return;
        };
        let initial = SavingsForm::from_account(account);
        self.open_form(initial, window, cx);
    }

    fn open_interest_accrual(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_some() || self.interest_form.is_some() {
            return;
        }
        let Some(account) = self
            .entries
            .iter()
            .find(|account| account.id == id)
            .cloned()
        else {
            return;
        };
        let categories =
            match finance_db::list_categories(&self.database, TransactionKind::Income, true) {
                Ok(categories) => categories,
                Err(error) => {
                    self.error(error.to_string(), cx);
                    return;
                }
            };
        let cards = match finance_db::list_payment_documents(
            &self.database,
            None,
            Some(PaymentCardType::Debit),
            None,
        ) {
            Ok(cards) => cards,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        };
        let category_entries: Vec<_> = category_tree_labels(&categories)
            .into_iter()
            .filter(|(category_id, _)| {
                categories
                    .iter()
                    .any(|category| category.id == *category_id && category.is_active)
            })
            .collect();
        let category_ids: Vec<i64> = category_entries
            .iter()
            .map(|(category_id, _)| *category_id)
            .collect();
        let category_id = category_ids.first().copied();
        let category_select = choice(
            category_entries
                .iter()
                .map(|(_, label)| label.clone())
                .collect(),
            (!category_ids.is_empty()).then_some(0),
            window,
            cx,
        );
        let accrual_date_value = account.interest_accrual_date;
        let accrual_date = cx.new(|cx| {
            let mut state = DatePickerState::new(window, cx).date_format(DATE_FORMAT);
            state.set_date(Date::Single(accrual_date_value), window, cx);
            state
        });
        let income_date = cx.new(|cx| {
            let mut state = DatePickerState::new(window, cx).date_format(DATE_FORMAT);
            state.set_date(Date::Single(accrual_date_value), window, cx);
            state
        });
        let interest_destination = account.interest_destination;
        let interest_destination_select = choice(
            InterestDestination::ALL
                .iter()
                .map(|destination| destination.title().to_owned())
                .collect(),
            InterestDestination::ALL
                .iter()
                .position(|destination| *destination == interest_destination),
            window,
            cx,
        );
        let income_currency = account.currency.unwrap_or(Currency::Rub);
        let income_currency_select = choice(
            Currency::ALL
                .iter()
                .map(|currency| currency.code().to_owned())
                .collect(),
            Currency::ALL
                .iter()
                .position(|currency| *currency == income_currency),
            window,
            cx,
        );
        let (recipient_options, recipient_labels) =
            interest_recipient_choices(&account, &cards, Some(income_currency));
        let recipient = Some(SavingsInterestIncomeRecipient::WithoutCard);
        let recipient_select = choice(recipient_labels, Some(0), window, cx);
        let (amount, error) = match default_interest_amount(&account, false) {
            Ok(amount) => (format!("{amount:.2}"), None),
            Err(error) => (
                String::new(),
                Some(format!("Не удалось рассчитать сумму по умолчанию: {error}")),
            ),
        };
        let initial_amount = amount.clone();
        let income_amount = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Например, 125,50");
            state.set_value(initial_amount, window, cx);
            state
        });
        let comment = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(3)
                .placeholder("Введите комментарий")
        });

        let mut subscriptions = vec![
            cx.subscribe_in(
                &income_date,
                window,
                |this: &mut Self, _state, _: &DatePickerEvent, _, cx| {
                    if let Some(form) = this.interest_form.as_mut() {
                        form.error = None;
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &income_amount,
                window,
                |this: &mut Self, state, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        if let Some(form) = this.interest_form.as_mut() {
                            form.manual_amount =
                                state.read(cx).value() != form.automatic_amount.as_str();
                            form.error = None;
                        }
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &interest_destination_select,
                window,
                |this: &mut Self, state, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(_) = event;
                    if let Some(form) = this.interest_form.as_mut() {
                        form.interest_destination = selected_choice_row(state, cx)
                            .and_then(|row| InterestDestination::ALL.get(row).copied())
                            .unwrap_or(form.interest_destination);
                        form.error = None;
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &category_select,
                window,
                |this: &mut Self, state, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(_) = event;
                    if let Some(form) = this.interest_form.as_mut() {
                        form.category_id = selected_choice_row(state, cx)
                            .and_then(|row| form.category_ids.get(row).copied());
                        form.error = None;
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &income_currency_select,
                window,
                |this: &mut Self, state, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let SelectEvent::Confirm(_) = event;
                    if let Some(form) = this.interest_form.as_mut() {
                        form.income_currency = selected_choice_row(state, cx)
                            .and_then(|row| Currency::ALL.get(row).copied());
                        form.error = None;
                    }
                    this.rebuild_interest_recipient_select(window, cx);
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &recipient_select,
                window,
                |this: &mut Self, state, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(_) = event;
                    if let Some(form) = this.interest_form.as_mut() {
                        form.recipient = selected_choice_row(state, cx)
                            .and_then(|row| form.recipient_options.get(row).copied());
                        form.error = None;
                    }
                    cx.notify();
                },
            ),
        ];
        subscriptions.shrink_to_fit();
        let mut form = InterestAccrualForm {
            account,
            accrual_date,
            interest_destination,
            interest_destination_select,
            create_income: false,
            income_date,
            income_amount,
            income_currency: Some(income_currency),
            income_currency_select,
            category_ids,
            category_id,
            category_select,
            cards,
            recipient_options,
            recipient,
            recipient_select,
            comment,
            higher_rate_applied: false,
            manual_amount: false,
            automatic_amount: amount.clone(),
            error,
            snapshot: InterestAccrualInput {
                accrual_date: accrual_date_value,
                interest_destination: Some(interest_destination),
                create_income: false,
                income_date: accrual_date_value,
                amount: amount.clone(),
                currency: Some(income_currency),
                category_id,
                recipient,
                comment: String::new(),
                higher_rate_applied: false,
                manual_amount: false,
            },
            confirm_discard: false,
            _subscriptions: subscriptions,
        };
        form.snapshot = form.input(cx);
        self.interest_form = Some(form);
        self.open_interest_accrual_dialog(window, cx);
    }

    fn rebuild_interest_recipient_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.interest_form.as_mut() else {
            return;
        };
        let (options, labels) =
            interest_recipient_choices(&form.account, &form.cards, form.income_currency);
        let selected = form
            .recipient
            .and_then(|recipient| options.iter().position(|option| *option == recipient))
            .unwrap_or(0);
        form.recipient = options.get(selected).copied();
        form.recipient_options = options;
        form.recipient_select.update(cx, |state, cx| {
            state.set_items(
                labels.into_iter().map(SharedString::from).collect(),
                window,
                cx,
            );
            state.set_selected_index(Some(IndexPath::default().row(selected)), window, cx);
        });
    }

    fn open_interest_accrual_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        window.open_dialog(cx, move |dialog: Dialog, _, cx| {
            let body = this.update(cx, |page, cx| page.render_interest_accrual_dialog(cx));
            dialog
                .title("Изменить выплату процентов")
                .centered()
                .w(px(800.0))
                .max_w(px(900.0))
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(body)
        });
    }

    fn open_form(&mut self, initial: SavingsForm, window: &mut Window, cx: &mut Context<Self>) {
        let text_input =
            |value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
                let value = value.to_owned();
                let placeholder = placeholder.to_owned();
                cx.new(|cx| {
                    let mut state = InputState::new(window, cx).placeholder(placeholder);
                    state.set_value(value, window, cx);
                    state
                })
            };
        let title = text_input(&initial.title, "", window, cx);
        let amount = text_input(&initial.amount, "Например, 100000", window, cx);
        let base_rate = text_input(&initial.base_rate, "Например, 5,5", window, cx);
        let threshold_amount = text_input(&initial.threshold_amount, "", window, cx);
        let rate_after_threshold = text_input(&initial.rate_after_threshold, "", window, cx);
        let date_input = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let date = NaiveDate::parse_from_str(value.trim(), DATE_FORMAT).ok();
            cx.new(|cx| {
                let mut state = DatePickerState::new(window, cx).date_format(DATE_FORMAT);
                state.set_date(Date::Single(date), window, cx);
                state
            })
        };
        let opened_date = date_input(&initial.opened_date, window, cx);
        let interest_accrual_date = date_input(&initial.interest_accrual_date, window, cx);
        let term_end_date = date_input(&initial.term_end_date, window, cx);

        let bank_items: Vec<String> = self.banks.iter().map(|bank| bank.name.clone()).collect();
        let bank_selected = initial
            .bank_id
            .and_then(|id| self.banks.iter().position(|bank| bank.id == id));
        let bank_select = choice(bank_items, bank_selected, window, cx);
        let currency_select = choice(
            currency_labels(NO_CURRENCY_LABEL),
            Some(
                initial
                    .currency
                    .and_then(|c| Currency::ALL.iter().position(|x| *x == c))
                    .map_or(0, |position| position + 1),
            ),
            window,
            cx,
        );
        let term_select = choice(
            TERMS.iter().map(|t| t.title().to_owned()).collect(),
            initial
                .term_type
                .and_then(|term| TERMS.iter().position(|t| *t == term)),
            window,
            cx,
        );
        let destination_select = choice(
            InterestDestination::ALL
                .iter()
                .map(|d| d.title().to_owned())
                .collect(),
            initial
                .interest_destination
                .and_then(|d| InterestDestination::ALL.iter().position(|x| *x == d)),
            window,
            cx,
        );

        let mut subscriptions = Vec::new();
        for picker in [&opened_date, &interest_accrual_date, &term_end_date] {
            subscriptions.push(cx.subscribe_in(
                picker,
                window,
                |_, _, _: &DatePickerEvent, _, cx| cx.notify(),
            ));
        }
        subscriptions.extend([
            cx.subscribe_in(
                &bank_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let bank_id = value.as_ref().and_then(|label| {
                        this.banks
                            .iter()
                            .find(|bank| bank.name == label.as_ref())
                            .map(|bank| bank.id)
                    });
                    if let Some(form) = this.form.as_mut() {
                        form.bank_id = bank_id;
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &currency_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(form) = this.form.as_mut() {
                        form.currency = value.as_ref().and_then(|l| currency_by_code(l.as_ref()));
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &term_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(form) = this.form.as_mut() {
                        form.term_type = value
                            .as_ref()
                            .and_then(|l| TERMS.into_iter().find(|t| t.title() == l.as_ref()));
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &destination_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(form) = this.form.as_mut() {
                        form.interest_destination = value.as_ref().and_then(|l| {
                            InterestDestination::ALL
                                .into_iter()
                                .find(|d| d.title() == l.as_ref())
                        });
                    }
                    cx.notify();
                },
            ),
        ]);

        self.form = Some(Form {
            id: initial.id,
            title,
            amount,
            base_rate,
            threshold_amount,
            rate_after_threshold,
            opened_date,
            interest_accrual_date,
            term_end_date,
            bank_select,
            currency_select,
            term_select,
            destination_select,
            bank_id: initial.bank_id,
            currency: initial.currency,
            term_type: initial.term_type,
            interest_destination: initial.interest_destination,
            use_threshold: initial.use_threshold,
            is_active: initial.is_active,
            snapshot: initial,
            error: None,
            _subscriptions: subscriptions,
        });
        self.confirm_discard = false;
        cx.notify();
        self.open_form_dialog(window, cx);
    }

    fn open_form_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        window.open_dialog(cx, move |dialog: Dialog, _, cx| {
            let (title, body) = this.update(cx, |page, cx| {
                let title = match page.form.as_ref().map(|form| form.id.is_some()) {
                    Some(true) => "Изменить накопление",
                    _ => "Новое накопление",
                };
                (title, page.render_form_dialog(cx))
            });
            dialog
                .title(title)
                .centered()
                .w(px(800.0))
                .max_w(px(900.0))
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(body)
        });
    }

    /// Текущие значения формы, собранные из полей ввода.
    fn read_form(&self, cx: &App) -> Option<SavingsForm> {
        let form = self.form.as_ref()?;
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let date_text = |picker: &Entity<DatePickerState>| match picker.read(cx).date() {
            Date::Single(Some(date)) => date.format(DATE_FORMAT).to_string(),
            _ => String::new(),
        };
        Some(SavingsForm {
            id: form.id,
            bank_id: form.bank_id,
            currency: form.currency,
            title: text(&form.title),
            amount: text(&form.amount),
            base_rate: text(&form.base_rate),
            use_threshold: form.use_threshold,
            rate_after_threshold: text(&form.rate_after_threshold),
            threshold_amount: text(&form.threshold_amount),
            opened_date: date_text(&form.opened_date),
            interest_accrual_date: date_text(&form.interest_accrual_date),
            term_type: form.term_type,
            term_end_date: date_text(&form.term_end_date),
            interest_destination: form.interest_destination,
            is_active: form.is_active,
        })
    }

    fn set_form_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(form) = self.form.as_mut() {
            form.error = Some(message);
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(values) = self.read_form(cx) else {
            return false;
        };
        let details = match values.details() {
            Ok(details) => details,
            Err(error) => {
                self.set_form_error(error, cx);
                return false;
            }
        };
        match finance_db::save_savings(&self.database, values.id, &details) {
            Ok(_) => {
                self.form = None;
                if self.refresh(cx) {
                    self.message = Some(("Накопление сохранено".to_owned(), false));
                }
                cx.notify();
                true
            }
            Err(error) => {
                self.set_form_error(error.to_string(), cx);
                false
            }
        }
    }

    fn save_interest_accrual(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = self.interest_form.as_ref() else {
            return false;
        };
        let account_id = form.account.id;
        let account = form.account.clone();
        let input = form.input(cx);
        let (accrual_date, interest_destination, income) =
            match build_interest_accrual_update(&input, &account) {
                Ok(update) => update,
                Err(error) => {
                    if let Some(form) = self.interest_form.as_mut() {
                        form.error = Some(error);
                    }
                    cx.notify();
                    return false;
                }
            };
        let created_income = income.is_some();
        match finance_db::save_savings_interest_accrual(
            &self.database,
            account_id,
            accrual_date,
            interest_destination,
            income,
        ) {
            Ok(_) => {
                self.interest_form = None;
                if self.refresh(cx) {
                    self.message = Some((
                        if created_income {
                            "Сведения о выплате процентов сохранены, доход сформирован".to_owned()
                        } else {
                            "Сведения о выплате процентов сохранены".to_owned()
                        },
                        false,
                    ));
                }
                cx.notify();
                true
            }
            Err(error) => {
                if let Some(form) = self.interest_form.as_mut() {
                    form.error = Some(error.to_string());
                }
                cx.notify();
                false
            }
        }
    }

    fn request_cancel_interest_accrual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self
            .interest_form
            .as_ref()
            .is_some_and(|form| form.input(cx) != form.snapshot);
        if dirty {
            if let Some(form) = self.interest_form.as_mut() {
                form.confirm_discard = true;
            }
        } else {
            self.interest_form = None;
            window.close_dialog(cx);
        }
        cx.notify();
    }

    fn render_interest_accrual_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.interest_form.as_ref() else {
            return div().into_any_element();
        };
        if form.confirm_discard {
            return self.render_interest_discard_confirmation(cx);
        }
        self.render_interest_form(cx)
    }

    fn render_interest_discard_confirmation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .child("Несохранённые изменения будут потеряны.")
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("interest-continue-editing")
                            .label("Вернуться к форме")
                            .on_click(cx.listener(|page, _, _, cx| {
                                if let Some(form) = page.interest_form.as_mut() {
                                    form.confirm_discard = false;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("interest-discard-changes")
                            .primary()
                            .label("Не сохранять")
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.interest_form = None;
                                window.close_dialog(cx);
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn set_higher_rate_applied(
        &mut self,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.interest_form.as_mut() else {
            return;
        };
        form.higher_rate_applied = checked;
        form.error = None;
        match interest_amount_for_rate_change(&form.account, checked, form.manual_amount) {
            Ok(Some(amount)) => {
                let amount = format!("{amount:.2}");
                form.automatic_amount = amount.clone();
                form.manual_amount = false;
                form.income_amount
                    .update(cx, |state, cx| state.set_value(amount, window, cx));
            }
            Ok(None) => {}
            Err(error) => form.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn render_interest_form(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.interest_form.as_ref() else {
            return div().into_any_element();
        };
        let danger = cx.theme().danger;
        let warning = cx.theme().warning;
        let field = |label: &'static str, width: f32, control: AnyElement| {
            div()
                .w(px(width))
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_size(px(12.0)).child(label))
                .child(control)
        };
        let row = || div().flex().items_end().gap_3();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(13.0))
                    .child(format!("Накопление: {}", form.account.title)),
            )
            .child(
                row()
                    .child(field(
                        "Дата начисления процентов",
                        250.0,
                        DatePicker::new(&form.accrual_date)
                            .placeholder("Выбрать дату")
                            .cleanable(true)
                            .into_any_element(),
                    ))
                    .child(field(
                        "Место начисления процентов",
                        350.0,
                        Select::new(&form.interest_destination_select).into_any_element(),
                    )),
            )
            .child(
                Checkbox::new("interest-create-income")
                    .label("Сформировать доход")
                    .checked(form.create_income)
                    .on_click(cx.listener(|page, checked: &bool, _, cx| {
                        if let Some(form) = page.interest_form.as_mut() {
                            form.create_income = *checked;
                            form.error = None;
                        }
                        cx.notify();
                    })),
            );

        if form.create_income {
            content = content
                .child(
                    row()
                        .child(field(
                            "Дата дохода",
                            230.0,
                            DatePicker::new(&form.income_date)
                                .placeholder("Выбрать дату")
                                .cleanable(true)
                                .into_any_element(),
                        ))
                        .child(field(
                            "Сумма",
                            230.0,
                            Input::new(&form.income_amount).into_any_element(),
                        ))
                        .child(field(
                            "Валюта",
                            180.0,
                            Select::new(&form.income_currency_select).into_any_element(),
                        )),
                )
                .child(
                    row()
                        .child(field(
                            "Категория дохода",
                            300.0,
                            Select::new(&form.category_select)
                                .placeholder("Выберите категорию")
                                .into_any_element(),
                        ))
                        .child(field(
                            "Получатель дохода",
                            350.0,
                            Select::new(&form.recipient_select)
                                .into_any_element(),
                        )),
                )
                .child(selectable_text(
                    "interest-form-calculation-note",
                    "Сумма по выбранной ставке: остаток × ставка / 100; даты и условия банка не проверяются.",
                ));

            if form.account.rate_after_threshold.is_some() {
                content = content.child(
                    Checkbox::new("interest-higher-rate-confirmation")
                        .label("Условия для повышенного процента выполнены")
                        .checked(form.higher_rate_applied)
                        .on_click(cx.listener(|page, checked: &bool, window, cx| {
                            page.set_higher_rate_applied(*checked, window, cx)
                        })),
                );
            }

            let savings_recipient_selected =
                form.recipient == Some(SavingsInterestIncomeRecipient::Savings);
            if savings_recipient_selected {
                let message = if !form.account.is_active {
                    Some("Пополнение неактивного накопления недоступно".to_owned())
                } else {
                    match form.account.currency {
                        None => {
                            Some("Сначала укажите валюту накопления в его настройках".to_owned())
                        }
                        Some(currency) if Some(currency) != form.income_currency => {
                            Some("Валюта накопления должна совпадать с валютой дохода".to_owned())
                        }
                        Some(_) => None,
                    }
                };
                if let Some(message) = message {
                    content = content.child(selectable_message(
                        "interest-recipient-currency-warning",
                        message,
                        warning,
                    ));
                }
            }

            content = content.child(field(
                "Комментарий",
                740.0,
                Textarea::new(&form.comment).into_any_element(),
            ));
        }

        if let Some(error) = &form.error {
            content = content.child(selectable_message(
                "interest-accrual-error",
                error.clone(),
                danger,
            ));
        }

        content = content.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("interest-accrual-save")
                        .primary()
                        .label("Сохранить")
                        .w(px(120.0))
                        .h(px(36.0))
                        .on_click(cx.listener(|page, _, window, cx| {
                            if page.save_interest_accrual(cx) {
                                window.close_dialog(cx);
                            }
                        })),
                )
                .child(
                    Button::new("interest-accrual-cancel")
                        .label("Отмена")
                        .w(px(120.0))
                        .h(px(36.0))
                        .on_click(cx.listener(|page, _, window, cx| {
                            page.request_cancel_interest_accrual(window, cx)
                        })),
                ),
        );
        content
            .p_4()
            .max_h(px(620.0))
            .overflow_y_scrollbar()
            .into_any_element()
    }

    fn request_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = match (self.read_form(cx), self.form.as_ref()) {
            (Some(current), Some(form)) => current != form.snapshot,
            _ => false,
        };
        if dirty {
            self.confirm_discard = true;
        } else {
            self.form = None;
            self.confirm_discard = false;
            window.close_dialog(cx);
        }
        cx.notify();
    }

    fn render_form_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.confirm_discard {
            return self.render_discard_confirmation(cx);
        }
        self.render_form(cx)
    }

    fn render_discard_confirmation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .child("Несохранённые изменения будут потеряны.")
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("savings-continue-editing")
                            .label("Вернуться к форме")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.confirm_discard = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("savings-discard-changes")
                            .primary()
                            .label("Не сохранять")
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.form = None;
                                page.confirm_discard = false;
                                window.close_dialog(cx);
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn confirm_delete(
        &mut self,
        id: i64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity();
        window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
            let this = this.clone();
            alert
                .title("Подтверждение удаления накопления")
                .description(format!("Удалить накопление «{title}»?"))
                .ok_text("Удалить")
                .cancel_text("Отмена")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |page, cx| {
                        match finance_db::delete_savings(&page.database, id) {
                            Ok(()) => {
                                if page.refresh(cx) {
                                    page.message = Some(("Накопление удалено".to_owned(), false));
                                }
                            }
                            Err(error) => page.error(error.to_string(), cx),
                        }
                    });
                    true
                })
        });
    }

    // ---- отрисовка ----

    fn render_filters(&mut self, cx: &mut Context<Self>) -> Div {
        let active = self.has_active_filters(cx);
        let filter = |label: &'static str, width: f32, control: AnyElement| {
            div()
                .w(px(width))
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_size(px(12.0)).child(label))
                .child(control)
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_2()
                    .child(filter(
                        "Поиск",
                        220.0,
                        Input::new(&self.search)
                            .cleanable(true)
                            .small()
                            .suffix(Icon::new(IconName::Search).small())
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Банк",
                        200.0,
                        Select::new(&self.bank_filter_select)
                            .placeholder("Банк")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Валюта",
                        110.0,
                        Select::new(&self.currency_filter_select)
                            .placeholder("Валюта")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Статус",
                        150.0,
                        Select::new(&self.active_filter_select)
                            .placeholder("Статус")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Срок",
                        180.0,
                        Select::new(&self.term_filter_select)
                            .placeholder("Срок")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Назначение процентов",
                        190.0,
                        Select::new(&self.destination_filter_select)
                            .placeholder("Назначение процентов")
                            .small()
                            .into_any_element(),
                    )),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("filters-apply")
                            .primary()
                            .small()
                            .label("Применить")
                            .on_click(cx.listener(|page, _, _, cx| page.apply_filters(cx))),
                    )
                    .child(
                        Button::new("filters-reset")
                            .small()
                            .label("Сбросить фильтры")
                            .disabled(!active)
                            .on_click(
                                cx.listener(|page, _, window, cx| page.reset_filters(window, cx)),
                            ),
                    ),
            )
    }

    fn render_form(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.form.as_ref() else {
            return div().into_any_element();
        };
        let danger = cx.theme().danger;
        let limited = form.term_type == Some(SavingsTerm::Limited);
        let labelled = |label: &'static str, width: f32, child: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .w(px(width))
                .child(div().text_size(px(12.0)).child(label))
                .child(child)
        };
        let row = || div().flex().items_end().gap_4();

        let mut fields = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                row()
                    .child(labelled(
                        "Название",
                        300.0,
                        Input::new(&form.title).into_any_element(),
                    ))
                    .child(labelled(
                        "Банк",
                        240.0,
                        Select::new(&form.bank_select)
                            .placeholder("Выберите банк")
                            .into_any_element(),
                    ))
                    .child(labelled(
                        "Валюта",
                        140.0,
                        Select::new(&form.currency_select).into_any_element(),
                    )),
            )
            .child(
                row()
                    .child(labelled(
                        "Сумма",
                        220.0,
                        Input::new(&form.amount).into_any_element(),
                    ))
                    .child(labelled(
                        "Базовая ставка, %",
                        220.0,
                        Input::new(&form.base_rate).into_any_element(),
                    ))
                    .child(labelled(
                        "Выплата процентов",
                        240.0,
                        Select::new(&form.destination_select)
                            .placeholder("Выберите назначение")
                            .into_any_element(),
                    )),
            )
            .child(
                row()
                    .child(
                        Checkbox::new("form-threshold")
                            .label("Есть повышение ставки при выполнении условий")
                            .checked(form.use_threshold)
                            .on_click(cx.listener(|page, checked: &bool, _, cx| {
                                if let Some(form) = page.form.as_mut() {
                                    form.use_threshold = *checked;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("form-active")
                            .label("Активно")
                            .checked(form.is_active)
                            .on_click(cx.listener(|page, checked: &bool, _, cx| {
                                if let Some(form) = page.form.as_mut() {
                                    form.is_active = *checked;
                                }
                                cx.notify();
                            })),
                    ),
            );
        if form.use_threshold {
            fields = fields.child(
                row()
                    .child(labelled(
                        "Пороговая сумма трат",
                        300.0,
                        Input::new(&form.threshold_amount).into_any_element(),
                    ))
                    .child(labelled(
                        "Ставка при выполнении условий, %",
                        300.0,
                        Input::new(&form.rate_after_threshold).into_any_element(),
                    )),
            );
        }
        fields = fields.child(
            row()
                .child(labelled(
                    "Дата открытия",
                    170.0,
                    DatePicker::new(&form.opened_date)
                        .placeholder("Выбрать дату")
                        .cleanable(true)
                        .into_any_element(),
                ))
                .child(labelled(
                    "Дата начисления %",
                    170.0,
                    DatePicker::new(&form.interest_accrual_date)
                        .placeholder("Выбрать дату")
                        .cleanable(true)
                        .into_any_element(),
                ))
                .child(labelled(
                    "Срок",
                    170.0,
                    Select::new(&form.term_select)
                        .placeholder("Выберите срок")
                        .into_any_element(),
                ))
                .when(limited, |r| {
                    r.child(labelled(
                        "Дата закрытия",
                        170.0,
                        DatePicker::new(&form.term_end_date)
                            .placeholder("Выбрать дату")
                            .cleanable(true)
                            .into_any_element(),
                    ))
                }),
        );

        let mut panel = div().flex().flex_col().gap_3().p_4().child(fields);
        if let Some(error) = form.error.clone() {
            panel = panel.child(selectable_message("savings-form-error", error, danger));
        }
        panel
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("form-save")
                            .primary()
                            .label("Сохранить")
                            .on_click(cx.listener(|page, _, window, cx| {
                                if page.save(cx) {
                                    page.confirm_discard = false;
                                    window.close_dialog(cx);
                                }
                            })),
                    )
                    .child(Button::new("form-cancel").label("Отмена").on_click(
                        cx.listener(|page, _, window, cx| page.request_cancel(window, cx)),
                    )),
            )
            .into_any_element()
    }

    fn render_table(&mut self, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(format!("Всего записей: {}", self.entries.len()))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true).bordered(true)),
            )
    }
}

fn column_widths_key() -> &'static str {
    SAVINGS_COLUMN_WIDTHS_KEY
}

fn format_column_widths(widths: &[f32]) -> String {
    widths
        .iter()
        .map(|width| width.round().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_column_widths(value: &str) -> Option<Vec<f32>> {
    Some(column_widths_with_fixed_actions(parse_width_values(
        value,
        COLUMN_HEADINGS.len(),
    )?))
}

fn parse_width_values(value: &str, count: usize) -> Option<Vec<f32>> {
    let widths = value
        .split(',')
        .map(|part| part.trim().parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()?;
    (widths.len() == count
        && widths.iter().enumerate().all(|(index, width)| {
            width.is_finite()
                && ((count == COLUMN_HEADINGS.len()
                    && index == COL_INTEREST_ACTION
                    && *width == INTEREST_ACTION_COLUMN_WIDTH)
                    || (MIN_COLUMN_WIDTH..=MAX_COLUMN_WIDTH).contains(width))
        }))
    .then_some(widths)
}

fn insert_interest_action_width(mut widths: Vec<f32>) -> Vec<f32> {
    widths.insert(COL_INTEREST_ACTION, INTEREST_ACTION_COLUMN_WIDTH);
    column_widths_with_fixed_actions(widths)
}

fn parse_v3_column_widths(value: &str) -> Option<Vec<f32>> {
    Some(insert_interest_action_width(parse_width_values(
        value,
        COLUMN_HEADINGS.len() - 1,
    )?))
}

/// Ширины V2 сохранялись по индексам; после перестановки `Название` и `Банк` возвращаем каждой её ширину.
fn parse_previous_column_widths(value: &str) -> Option<Vec<f32>> {
    let mut widths = parse_width_values(value, COLUMN_HEADINGS.len() - 1)?;
    widths.swap(COL_TITLE, COL_BANK);
    Some(insert_interest_action_width(widths))
}

fn parse_legacy_column_widths(value: &str) -> Option<Vec<f32>> {
    let mut widths = parse_width_values(value, COLUMN_HEADINGS.len() - 1)?;
    widths.swap(COL_TITLE, COL_BANK);
    widths.swap(COL_OPENED_DATE, COL_TERM);
    Some(insert_interest_action_width(widths))
}

fn column_widths_with_fixed_actions(mut widths: Vec<f32>) -> Vec<f32> {
    if let Some(width) = widths.get_mut(COL_INTEREST_ACTION) {
        *width = INTEREST_ACTION_COLUMN_WIDTH;
    }
    if let Some(width) = widths.get_mut(COL_ACTIONS) {
        *width = ACTIONS_COLUMN_WIDTH;
    }
    widths
}

fn load_column_widths(database: &Database) -> Vec<f32> {
    const INVALID_WIDTHS: &str =
        "Некорректная сохранённая ширина колонок накоплений; использованы значения по умолчанию";
    const READ_ERROR: &str = "Не удалось прочитать ширину колонок накоплений:";

    match finance_db::get_app_setting(database, column_widths_key()) {
        Ok(Some(value)) => parse_column_widths(&value)
            .map(column_widths_with_fixed_actions)
            .unwrap_or_else(|| {
                eprintln!("{INVALID_WIDTHS}");
                DEFAULT_COLUMN_WIDTHS.to_vec()
            }),
        Ok(None) => match finance_db::get_app_setting(database, PREVIOUS_SAVINGS_COLUMN_WIDTHS_KEY)
        {
            Ok(Some(value)) => parse_v3_column_widths(&value).unwrap_or_else(|| {
                eprintln!("{INVALID_WIDTHS}");
                DEFAULT_COLUMN_WIDTHS.to_vec()
            }),
            Ok(None) => {
                match finance_db::get_app_setting(database, OLDER_SAVINGS_COLUMN_WIDTHS_KEY) {
                    Ok(Some(value)) => parse_previous_column_widths(&value).unwrap_or_else(|| {
                        eprintln!("{INVALID_WIDTHS}");
                        DEFAULT_COLUMN_WIDTHS.to_vec()
                    }),
                    Ok(None) => {
                        match finance_db::get_app_setting(
                            database,
                            LEGACY_SAVINGS_COLUMN_WIDTHS_KEY,
                        ) {
                            Ok(Some(value)) => {
                                parse_legacy_column_widths(&value).unwrap_or_else(|| {
                                    eprintln!("{INVALID_WIDTHS}");
                                    DEFAULT_COLUMN_WIDTHS.to_vec()
                                })
                            }
                            Ok(None) => DEFAULT_COLUMN_WIDTHS.to_vec(),
                            Err(error) => {
                                eprintln!("{READ_ERROR} {error}");
                                DEFAULT_COLUMN_WIDTHS.to_vec()
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("{READ_ERROR} {error}");
                        DEFAULT_COLUMN_WIDTHS.to_vec()
                    }
                }
            }
            Err(error) => {
                eprintln!("{READ_ERROR} {error}");
                DEFAULT_COLUMN_WIDTHS.to_vec()
            }
        },
        Err(error) => {
            eprintln!("{READ_ERROR} {error}");
            DEFAULT_COLUMN_WIDTHS.to_vec()
        }
    }
}

fn default_order(a: &SavingsAccount, b: &SavingsAccount) -> Ordering {
    a.bank_name
        .to_lowercase()
        .cmp(&b.bank_name.to_lowercase())
        .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        .then(a.id.cmp(&b.id))
}

fn compare_terms(a: &SavingsAccount, b: &SavingsAccount) -> Ordering {
    let term_order = |account: &SavingsAccount| match account.term_type {
        SavingsTerm::Limited => 0,
        SavingsTerm::Unlimited => 1,
    };
    term_order(a)
        .cmp(&term_order(b))
        .then(a.term_end_date.cmp(&b.term_end_date))
}

fn sort_savings(rows: &mut [SavingsAccount], sort: Option<(usize, ColumnSort)>) {
    let Some((column, direction)) = sort else {
        rows.sort_by(default_order);
        return;
    };
    rows.sort_by(|a, b| {
        let ordering = match column {
            COL_BANK => a.bank_name.to_lowercase().cmp(&b.bank_name.to_lowercase()),
            COL_TITLE => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            COL_AMOUNT => a.amount.cmp(&b.amount),
            COL_BASE_RATE => a.base_rate.cmp(&b.base_rate),
            COL_RATE_AFTER_THRESHOLD => a.rate_after_threshold.cmp(&b.rate_after_threshold),
            COL_THRESHOLD => a.threshold_amount.cmp(&b.threshold_amount),
            COL_OPENED_DATE => a.opened_date.cmp(&b.opened_date),
            COL_TERM => compare_terms(a, b),
            COL_INTEREST_DATE => a.interest_accrual_date.cmp(&b.interest_accrual_date),
            _ => Ordering::Equal,
        };
        let ordering = if direction == ColumnSort::Descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| default_order(a, b))
    });
}

struct SavingsTable {
    page: WeakEntity<SavingsPage>,
    rows: Vec<SavingsAccount>,
    widths: Vec<f32>,
    sort: Option<(usize, ColumnSort)>,
}

impl SavingsTable {
    fn new(page: WeakEntity<SavingsPage>, widths: Vec<f32>) -> Self {
        Self {
            page,
            rows: Vec::new(),
            widths: column_widths_with_fixed_actions(widths),
            sort: None,
        }
    }

    fn set_rows(&mut self, rows: Vec<SavingsAccount>) {
        self.rows = rows;
        sort_savings(&mut self.rows, self.sort);
    }

    fn set_widths(&mut self, widths: &[f32]) {
        if widths.len() == self.widths.len() {
            self.widths = column_widths_with_fixed_actions(widths.to_vec());
        }
    }
}

impl TableDelegate for SavingsTable {
    fn columns_count(&self, _: &App) -> usize {
        COLUMN_HEADINGS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn group_headers(&self, _: &App) -> Option<Vec<Vec<ColumnGroup>>> {
        Some(savings_group_headers())
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let border = cx.theme().border;
        div()
            .w_full()
            .h_full()
            .when(col_ix > 0, |cell| cell.border_l_1().border_color(border))
            .child(COLUMN_HEADINGS[col_ix])
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let is_actions = col_ix == COL_ACTIONS;
        let is_interest_action = col_ix == COL_INTEREST_ACTION;
        let is_fixed_width = is_actions || is_interest_action;
        let action_width = if is_interest_action {
            INTEREST_ACTION_COLUMN_WIDTH
        } else {
            ACTIONS_COLUMN_WIDTH
        };
        let column = Column::new(COLUMN_HEADINGS[col_ix], COLUMN_HEADINGS[col_ix])
            .width(px(self.widths[col_ix]))
            .min_width(px(if is_fixed_width {
                action_width
            } else {
                MIN_COLUMN_WIDTH
            }))
            .max_width(px(if is_fixed_width {
                action_width
            } else {
                MAX_COLUMN_WIDTH
            }))
            .resizable(!is_fixed_width)
            .movable(false);
        let column = if is_actions {
            column.fixed(ColumnFixed::Right)
        } else {
            column
        };
        let direction = match self.sort {
            Some((sorted, direction)) if sorted == col_ix => direction,
            _ => ColumnSort::Default,
        };
        match col_ix {
            COL_BANK
            | COL_TITLE
            | COL_AMOUNT
            | COL_BASE_RATE
            | COL_RATE_AFTER_THRESHOLD
            | COL_THRESHOLD
            | COL_OPENED_DATE
            | COL_TERM
            | COL_INTEREST_DATE => column.sort(direction),
            _ => column,
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.sort = (sort != ColumnSort::Default).then_some((col_ix, sort));
        sort_savings(&mut self.rows, self.sort);
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(selectable_text(
                "savings-empty",
                "Накоплений по выбранным условиям нет.",
            ))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let account = &self.rows[row_ix];
        let dash = || "—".to_owned();
        let term = match (account.term_type, account.term_end_date) {
            (SavingsTerm::Limited, Some(date)) => format!("до {}", format_date(date)),
            (SavingsTerm::Limited, None) => "Дата не задана".to_owned(),
            (SavingsTerm::Unlimited, _) => "Бессрочный".to_owned(),
        };
        let content = match col_ix {
            COL_BANK => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account.bank_name.clone(),
                ))
                .into_any_element(),
            COL_TITLE => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account.title.clone(),
                ))
                .into_any_element(),
            COL_AMOUNT => selectable_right_aligned_table_cell(
                "savings",
                account.id,
                col_ix,
                format_currency(&account.amount, 2),
            )
            .into_any_element(),
            3 => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account
                        .currency
                        .map_or_else(dash, |currency| currency.code().to_owned()),
                ))
                .into_any_element(),
            COL_BASE_RATE => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    format!("{}%", account.base_rate),
                ))
                .into_any_element(),
            COL_RATE_AFTER_THRESHOLD => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account
                        .rate_after_threshold
                        .map_or_else(dash, |rate| format!("{rate}%")),
                ))
                .into_any_element(),
            COL_THRESHOLD => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account
                        .threshold_amount
                        .map_or_else(dash, |amount| format_currency(&amount, 2)),
                ))
                .into_any_element(),
            COL_TERM => div()
                .child(selectable_table_cell("savings", account.id, col_ix, term))
                .into_any_element(),
            COL_OPENED_DATE => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account.opened_date.map_or_else(dash, format_date),
                ))
                .into_any_element(),
            COL_INTEREST_DATE => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account.interest_accrual_date.map_or_else(dash, format_date),
                ))
                .into_any_element(),
            10 => div()
                .child(selectable_table_cell(
                    "savings",
                    account.id,
                    col_ix,
                    account.interest_destination.title().to_owned(),
                ))
                .into_any_element(),
            COL_INTEREST_ACTION => {
                let id = account.id;
                let page = self.page.clone();
                div()
                    .flex()
                    .w_full()
                    .justify_center()
                    .child(
                        Button::new(("savings-interest-accrual", id as usize))
                            .xsmall()
                            .icon(IconName::Inspector)
                            .on_click(move |_, window, cx| {
                                let _ = page.update(cx, |page, cx| {
                                    page.open_interest_accrual(id, window, cx)
                                });
                            }),
                    )
                    .into_any_element()
            }
            COL_ACTIONS => {
                let id = account.id;
                let title = account.title.clone();
                let edit_page = self.page.clone();
                let delete_page = self.page.clone();
                div()
                    .flex()
                    .w_full()
                    .justify_center()
                    .gap_1()
                    .child(
                        Button::new(("edit", id as usize))
                            .xsmall()
                            .label("Изменить")
                            .on_click(move |_, window, cx| {
                                let _ = edit_page
                                    .update(cx, |page, cx| page.begin_edit(id, window, cx));
                            }),
                    )
                    .child(
                        Button::new(("delete", id as usize))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .on_click(move |_, window, cx| {
                                let _ = delete_page.update(cx, |page, cx| {
                                    page.confirm_delete(id, title.clone(), window, cx)
                                });
                            }),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        };
        let border = cx.theme().border;
        div()
            .w_full()
            .h_full()
            .when(col_ix > 0, |cell| cell.border_l_1().border_color(border))
            .child(content)
            .into_any_element()
    }
}

impl Render for SavingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let success = cx.theme().success;
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let mut root = div().size_full().flex().flex_col().gap_2().p_3();
        root = root.child(
            div()
                .flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(24.0))
                        .font_weight(FontWeight::BOLD)
                        .child("Накопления"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("new-savings")
                                .primary()
                                .large()
                                .label("Добавить накопление")
                                .icon(IconName::Plus)
                                .disabled(self.form.is_some() || self.banks.is_empty())
                                .on_click(
                                    cx.listener(|page, _, window, cx| page.begin_new(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("refresh-savings")
                                .secondary()
                                .large()
                                .label("Обновить")
                                .icon(IconName::Replace)
                                .on_click(
                                    cx.listener(|page, _, window, cx| page.reload(window, cx)),
                                ),
                        ),
                ),
        );
        root = root.child(self.render_filters(cx));
        if self.banks.is_empty() {
            root = root.child(selectable_text(
                "savings-missing-bank",
                "Сначала добавьте банк в разделе «Справочники», чтобы создать накопление.",
            ));
        }
        if let Some((message, is_error)) = &self.message {
            root = root.child(selectable_message(
                "savings-page-message",
                message.clone(),
                if *is_error { danger } else { success },
            ));
        }
        root = root.child(self.render_table(cx));
        root.child(selectable_message(
            "savings-footer-note",
            FOOTER_NOTE,
            muted,
        ))
        .child(selectable_message(
            "savings-footer-warning",
            FOOTER_WARNING,
            warning,
        ))
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use finance_core::{
        Currency, InterestDestination, PaymentCardType, PaymentDocument, PaymentSystem,
        SavingsAccount, SavingsTerm,
    };
    use finance_db::SavingsInterestIncomeRecipient;
    use gpui_kit::component::table::ColumnSort;
    use rust_decimal::Decimal;

    use super::{
        build_interest_accrual_update, column_widths_key, column_widths_with_fixed_actions,
        default_interest_amount, destination_filter_labels, filter_savings, format_column_widths,
        interest_amount_for_rate_change, interest_recipient_choices, load_column_widths,
        parse_column_widths, parse_legacy_column_widths, parse_previous_column_widths,
        parse_v3_column_widths, savings_group_headers, sort_savings, term_filter_labels,
        tooltip_items, InterestAccrualInput, SavingsForm, ACTIONS_COLUMN_WIDTH,
        BANK_FILTER_ITEM_ID, COLUMN_HEADINGS, COL_ACTIONS, COL_AMOUNT, COL_BANK, COL_BASE_RATE,
        COL_INTEREST_ACTION, COL_INTEREST_DATE, COL_OPENED_DATE, COL_RATE_AFTER_THRESHOLD,
        COL_TERM, COL_THRESHOLD, COL_TITLE, DEFAULT_COLUMN_WIDTHS, INTEREST_ACTION_COLUMN_WIDTH,
        INTEREST_GROUP_LABEL, INTEREST_GROUP_SPAN, LEGACY_SAVINGS_COLUMN_WIDTHS_KEY,
        OLDER_SAVINGS_COLUMN_WIDTHS_KEY, PREVIOUS_SAVINGS_COLUMN_WIDTHS_KEY, RATE_GROUP_LABEL,
        RATE_GROUP_SPAN, SAVINGS_COLUMN_WIDTHS_KEY,
    };

    const DEFAULT_V3_COLUMN_WIDTHS: [f32; 12] = [
        120.0, 85.0, 90.0, 70.0, 80.0, 170.0, 90.0, 95.0, 115.0, 105.0, 120.0, 160.0,
    ];

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

    fn interest_account() -> SavingsAccount {
        let mut account = sortable_savings()[0].clone();
        account.amount = Decimal::from(1_000);
        account.base_rate = Decimal::new(55, 1);
        account.rate_after_threshold = Some(Decimal::new(725, 2));
        account.currency = Some(Currency::Rub);
        account
    }

    fn interest_accrual_input() -> InterestAccrualInput {
        InterestAccrualInput {
            accrual_date: NaiveDate::from_ymd_opt(2026, 11, 25),
            interest_destination: Some(InterestDestination::Withdraw),
            create_income: false,
            income_date: NaiveDate::from_ymd_opt(2026, 11, 24),
            amount: "12.34".to_owned(),
            currency: Some(Currency::Rub),
            category_id: Some(42),
            recipient: Some(SavingsInterestIncomeRecipient::WithoutCard),
            comment: "Комментарий".to_owned(),
            higher_rate_applied: false,
            manual_amount: false,
        }
    }

    fn payment_document(id: i64, currency: Currency) -> PaymentDocument {
        PaymentDocument {
            id,
            name: Some(format!("Карта {id}")),
            bank_id: 1,
            bank_name: "Банк".to_owned(),
            payment_system: PaymentSystem::Mir,
            card_type: PaymentCardType::Debit,
            amount: Decimal::ZERO,
            currency: Some(currency),
            due_date: None,
            grace_period_days: None,
            next_payment_amount: None,
            affects_deposit_rate: false,
            min_monthly_spend: None,
            linked_savings: Vec::new(),
        }
    }

    #[test]
    fn interest_default_uses_selected_rate_without_date_factor() {
        let account = interest_account();

        assert_eq!(
            default_interest_amount(&account, false),
            Ok(Decimal::from(55))
        );
        assert_eq!(
            default_interest_amount(&account, true),
            Ok(Decimal::new(725, 1))
        );
    }

    #[test]
    fn rate_change_does_not_replace_manually_edited_amount() {
        let account = interest_account();

        assert_eq!(
            interest_amount_for_rate_change(&account, true, true),
            Ok(None)
        );
    }

    #[test]
    fn interest_accrual_without_income_only_builds_reference_fields() {
        let account = interest_account();
        let mut input = interest_accrual_input();
        input.amount = "invalid hidden amount".to_owned();
        input.currency = None;
        input.category_id = None;
        input.recipient = None;

        let update = build_interest_accrual_update(&input, &account).unwrap();

        assert_eq!(
            update,
            (
                Some(NaiveDate::from_ymd_opt(2026, 11, 25).unwrap()),
                InterestDestination::Withdraw,
                None
            )
        );
    }

    #[test]
    fn interest_accrual_can_keep_the_reference_date_empty_without_income() {
        let account = interest_account();
        let mut input = interest_accrual_input();
        input.accrual_date = None;

        let update = build_interest_accrual_update(&input, &account).unwrap();

        assert_eq!(update.0, None);
    }

    #[test]
    fn interest_accrual_builds_an_ordinary_income_with_selected_fields() {
        let account = interest_account();
        let mut input = interest_accrual_input();
        input.create_income = true;

        let (_, _, income) = build_interest_accrual_update(&input, &account).unwrap();

        let income = income.unwrap();
        assert_eq!(income.date, NaiveDate::from_ymd_opt(2026, 11, 24).unwrap());
        assert_eq!(income.category_id, 42);
        assert_eq!(income.amount, Decimal::new(1234, 2));
        assert_eq!(income.currency, Currency::Rub);
        assert_eq!(income.comment, "Комментарий");
        assert_eq!(
            income.recipient,
            SavingsInterestIncomeRecipient::WithoutCard
        );
    }

    #[test]
    fn interest_accrual_rejects_savings_recipient_without_matching_currency() {
        let mut account = interest_account();
        let mut input = interest_accrual_input();
        input.create_income = true;
        input.recipient = Some(SavingsInterestIncomeRecipient::Savings);
        account.currency = None;

        let error = build_interest_accrual_update(&input, &account).unwrap_err();

        assert!(error.contains("сначала укажите его валюту"));
    }

    #[test]
    fn interest_accrual_rejects_savings_recipient_with_a_different_currency() {
        let account = interest_account();
        let mut input = interest_accrual_input();
        input.create_income = true;
        input.recipient = Some(SavingsInterestIncomeRecipient::Savings);
        input.currency = Some(Currency::Usd);

        let error = build_interest_accrual_update(&input, &account).unwrap_err();

        assert!(error.contains("должна совпадать"));
    }

    #[test]
    fn interest_accrual_recipient_list_filters_cards_by_income_currency() {
        let account = interest_account();
        let cards = [
            payment_document(1, Currency::Rub),
            payment_document(2, Currency::Usd),
        ];

        let (recipients, labels) =
            interest_recipient_choices(&account, &cards, Some(Currency::Rub));

        assert_eq!(
            recipients,
            [
                SavingsInterestIncomeRecipient::WithoutCard,
                SavingsInterestIncomeRecipient::PaymentDocument(1),
                SavingsInterestIncomeRecipient::Savings,
            ]
        );
        assert_eq!(labels[0], "Без карты");
        assert!(labels[1].contains("Карта 1"));
        assert!(labels[2].contains(&account.title));
    }

    #[test]
    fn savings_form_parses_optional_dates_and_allows_them_to_be_empty() {
        let mut form = valid_savings_form();
        form.opened_date = "01.03.2026".to_owned();
        form.interest_accrual_date = "31.03.2026".to_owned();

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
        form.opened_date = "30.02.2026".to_owned();
        assert!(form.details().unwrap_err().contains("дату открытия"));

        form.opened_date.clear();
        form.interest_accrual_date = "not-a-date".to_owned();
        assert!(form
            .details()
            .unwrap_err()
            .contains("дату начисления процентов"));
    }

    #[test]
    fn savings_form_rejects_invalid_amount_and_missing_bank() {
        let mut form = valid_savings_form();
        form.amount = "не число".to_owned();
        assert!(form.details().is_err());

        let mut form = valid_savings_form();
        form.bank_id = None;
        assert_eq!(form.details().unwrap_err(), "Выберите банк");
    }

    #[test]
    fn limited_term_requires_end_date_and_unlimited_drops_it() {
        let mut form = valid_savings_form();
        form.term_type = Some(SavingsTerm::Limited);
        assert!(form.details().is_err());
        form.term_end_date = "01.03.2027".to_owned();
        assert_eq!(
            form.details().unwrap().term_end_date,
            NaiveDate::from_ymd_opt(2027, 3, 1)
        );
        form.term_type = Some(SavingsTerm::Unlimited);
        assert_eq!(form.details().unwrap().term_end_date, None);
    }

    #[test]
    fn threshold_values_are_parsed_only_when_enabled() {
        let mut form = valid_savings_form();
        form.threshold_amount = "мусор".to_owned();
        assert_eq!(form.details().unwrap().threshold_amount, None);

        form.use_threshold = true;
        assert!(form.details().is_err());
        form.threshold_amount = "30000".to_owned();
        form.rate_after_threshold = "7".to_owned();
        let details = form.details().unwrap();
        assert_eq!(details.threshold_amount.unwrap().to_string(), "30000");
        assert_eq!(details.rate_after_threshold.unwrap().to_string(), "7");
    }

    #[test]
    fn new_form_has_expected_defaults_and_differs_after_edit() {
        let banks = [finance_core::Bank {
            id: 7,
            name: "Банк".to_owned(),
            comment: String::new(),
        }];
        let initial = SavingsForm::new_for(&banks);
        assert_eq!(initial.bank_id, Some(7));
        assert_eq!(initial.base_rate, "0");
        assert!(initial.is_active);
        let mut edited = initial.clone();
        assert!(edited == initial);
        edited.title = "Черновик".to_owned();
        assert!(edited != initial);
    }

    #[test]
    fn interest_destination_options_include_other() {
        assert_eq!(
            InterestDestination::ALL.map(|destination| destination.title()),
            ["Капитализация", "На карту", "Другое"]
        );
    }

    #[test]
    fn saved_other_interest_destination_is_displayed_and_loaded_into_edit_form() {
        let database = filtered_database();
        let account = finance_db::list_savings_filtered(&database, None, None, None, "")
            .expect("savings should load")
            .into_iter()
            .find(|account| account.id == 6)
            .expect("other-destination savings should load");

        assert_eq!(account.interest_destination.title(), "Другое");
        let form = SavingsForm::from_account(&account);
        assert_eq!(form.interest_destination, Some(InterestDestination::Other));
        assert_eq!(
            form.details()
                .expect("saved other destination should be editable")
                .interest_destination,
            InterestDestination::Other
        );
    }

    fn sortable_savings() -> Vec<SavingsAccount> {
        vec![
            SavingsAccount {
                id: 1,
                bank_id: 1,
                bank_name: "Банк Бета".to_owned(),
                title: "Зета".to_owned(),
                amount: Decimal::from(2),
                currency: None,
                base_rate: Decimal::from(7),
                rate_after_threshold: Some(Decimal::from(8)),
                threshold_amount: Some(Decimal::from(100)),
                opened_date: NaiveDate::from_ymd_opt(2026, 2, 1),
                interest_accrual_date: None,
                term_type: SavingsTerm::Limited,
                term_end_date: NaiveDate::from_ymd_opt(2030, 1, 1),
                interest_destination: InterestDestination::Reinvest,
                is_active: true,
            },
            SavingsAccount {
                id: 2,
                bank_id: 2,
                bank_name: "Банк Альфа".to_owned(),
                title: "Альфа".to_owned(),
                amount: Decimal::from(10),
                currency: None,
                base_rate: Decimal::from(5),
                rate_after_threshold: None,
                threshold_amount: None,
                opened_date: None,
                interest_accrual_date: None,
                term_type: SavingsTerm::Unlimited,
                term_end_date: None,
                interest_destination: InterestDestination::Reinvest,
                is_active: true,
            },
            SavingsAccount {
                id: 3,
                bank_id: 2,
                bank_name: "Банк Альфа".to_owned(),
                title: "Гамма".to_owned(),
                amount: Decimal::from(1),
                currency: None,
                base_rate: Decimal::from(9),
                rate_after_threshold: Some(Decimal::from(6)),
                threshold_amount: Some(Decimal::from(50)),
                opened_date: NaiveDate::from_ymd_opt(2026, 1, 1),
                interest_accrual_date: None,
                term_type: SavingsTerm::Limited,
                term_end_date: NaiveDate::from_ymd_opt(2028, 1, 1),
                interest_destination: InterestDestination::Withdraw,
                is_active: true,
            },
        ]
    }

    fn sorted_ids(column: usize, direction: ColumnSort) -> Vec<i64> {
        let mut rows = sortable_savings();
        sort_savings(&mut rows, Some((column, direction)));
        rows.into_iter().map(|account| account.id).collect()
    }

    #[test]
    fn savings_sorting_covers_all_requested_columns_in_both_directions() {
        let columns = [
            (COL_BANK, [2, 3, 1], [1, 2, 3]),
            (COL_TITLE, [2, 3, 1], [1, 3, 2]),
            (COL_AMOUNT, [3, 1, 2], [2, 1, 3]),
            (COL_BASE_RATE, [2, 1, 3], [3, 1, 2]),
            (COL_RATE_AFTER_THRESHOLD, [2, 3, 1], [1, 3, 2]),
            (COL_THRESHOLD, [2, 3, 1], [1, 3, 2]),
            (COL_OPENED_DATE, [2, 3, 1], [1, 3, 2]),
            (COL_TERM, [3, 1, 2], [2, 1, 3]),
        ];

        for (column, ascending, descending) in columns {
            assert_eq!(
                sorted_ids(column, ColumnSort::Ascending),
                ascending,
                "ascending sort for column {column}"
            );
            assert_eq!(
                sorted_ids(column, ColumnSort::Descending),
                descending,
                "descending sort for column {column}"
            );
        }
    }

    #[test]
    fn savings_column_order_places_opening_date_before_term() {
        assert_eq!(COL_OPENED_DATE, 7);
        assert_eq!(COL_TERM, 8);
        assert_eq!(COLUMN_HEADINGS[COL_OPENED_DATE], "Дата открытия");
        assert_eq!(COLUMN_HEADINGS[COL_TERM], "Срок");
    }

    #[test]
    fn savings_headers_group_rates_and_interest_accrual() {
        assert_eq!(COLUMN_HEADINGS[COL_BASE_RATE], "Базовая");
        assert_eq!(COLUMN_HEADINGS[COL_RATE_AFTER_THRESHOLD], "Повышенная");
        assert_eq!(COLUMN_HEADINGS[COL_THRESHOLD], "Условие");
        assert_eq!(COLUMN_HEADINGS[COL_INTEREST_DATE], "Дата");
        assert_eq!(COLUMN_HEADINGS[COL_INTEREST_ACTION - 1], "Начисление");
        assert_eq!(COLUMN_HEADINGS[COL_INTEREST_ACTION], "");

        let rows = savings_group_headers();
        assert_eq!(rows.len(), 1);
        let groups = &rows[0];
        assert_eq!(
            groups.iter().map(|group| group.span).sum::<usize>(),
            COLUMN_HEADINGS.len()
        );
        assert_eq!(
            groups
                .iter()
                .take_while(|group| group.label.is_empty())
                .map(|group| group.span)
                .sum::<usize>(),
            COL_BASE_RATE
        );
        assert_eq!(groups[1].label, RATE_GROUP_LABEL);
        assert_eq!(groups[1].span, RATE_GROUP_SPAN);
        assert_eq!(
            COL_BASE_RATE + groups[1].span + groups[2].span,
            COL_INTEREST_DATE
        );
        assert_eq!(groups[3].label, INTEREST_GROUP_LABEL);
        assert_eq!(groups[3].span, INTEREST_GROUP_SPAN);
        assert_eq!(INTEREST_GROUP_SPAN, 3);
        assert_eq!(
            DEFAULT_COLUMN_WIDTHS[COL_INTEREST_ACTION],
            INTEREST_ACTION_COLUMN_WIDTH
        );
        assert_eq!(COL_ACTIONS, COLUMN_HEADINGS.len() - 1);
    }

    #[test]
    fn savings_column_order_places_title_before_bank() {
        assert_eq!(COL_TITLE, 0);
        assert_eq!(COL_BANK, 1);
        assert_eq!(COLUMN_HEADINGS[COL_TITLE], "Название");
        assert_eq!(COLUMN_HEADINGS[COL_BANK], "Банк");
        assert_eq!(DEFAULT_COLUMN_WIDTHS[COL_TITLE], 120.0);
        assert_eq!(DEFAULT_COLUMN_WIDTHS[COL_BANK], 85.0);
    }

    #[test]
    fn savings_filter_tooltip_items_keep_labels_and_have_unique_ids() {
        let labels = vec!["Все банки".to_owned(), "Банк".to_owned()];

        let items = tooltip_items(labels.clone(), BANK_FILTER_ITEM_ID);

        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            labels.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(items.iter().map(|item| item.id).collect::<Vec<_>>(), [0, 1]);
        assert!(items.iter().all(|item| item.item_id == BANK_FILTER_ITEM_ID));
    }

    #[test]
    fn savings_filters_match_each_value_combine_and_reset() {
        let mut entries = sortable_savings();
        let mut other = sortable_savings().into_iter().next().unwrap();
        other.id = 4;
        other.term_type = SavingsTerm::Unlimited;
        other.interest_destination = InterestDestination::Other;
        entries.push(other);
        let ids =
            |rows: Vec<SavingsAccount>| rows.into_iter().map(|row| row.id).collect::<Vec<_>>();

        assert_eq!(
            term_filter_labels(),
            ["Все", "С ограниченным сроком", "Бессрочный"]
        );
        assert_eq!(
            destination_filter_labels(),
            ["Все", "Капитализация", "На карту", "Другое"]
        );
        assert_eq!(
            ids(filter_savings(
                entries.clone(),
                Some(SavingsTerm::Limited),
                None
            )),
            [1, 3]
        );
        assert_eq!(
            ids(filter_savings(
                entries.clone(),
                Some(SavingsTerm::Unlimited),
                None
            )),
            [2, 4]
        );
        assert_eq!(
            ids(filter_savings(
                entries.clone(),
                None,
                Some(InterestDestination::Reinvest)
            )),
            [1, 2]
        );
        assert_eq!(
            ids(filter_savings(
                entries.clone(),
                None,
                Some(InterestDestination::Withdraw)
            )),
            [3]
        );
        assert_eq!(
            ids(filter_savings(
                entries.clone(),
                None,
                Some(InterestDestination::Other)
            )),
            [4]
        );
        assert_eq!(
            ids(filter_savings(
                entries.clone(),
                Some(SavingsTerm::Limited),
                Some(InterestDestination::Withdraw)
            )),
            [3]
        );
        assert_eq!(
            ids(filter_savings(entries.clone(), None, None)),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn savings_column_widths_are_loaded_from_database_and_validated() {
        let database = filtered_database();
        let mut widths = DEFAULT_COLUMN_WIDTHS;
        widths[COL_TITLE] = 245.0;
        widths[COL_ACTIONS] = 235.0;
        finance_db::set_app_setting(
            &database,
            column_widths_key(),
            &format_column_widths(&widths),
        )
        .expect("column widths should be stored");

        widths[COL_ACTIONS] = ACTIONS_COLUMN_WIDTH;
        assert_eq!(load_column_widths(&database), widths);
    }

    #[test]
    fn v3_savings_column_widths_migrate_to_v4_without_losing_user_widths() {
        let database = filtered_database();
        let mut previous_widths = DEFAULT_V3_COLUMN_WIDTHS;
        previous_widths[COL_TITLE] = 245.0;
        previous_widths[11] = 235.0;
        finance_db::set_app_setting(
            &database,
            PREVIOUS_SAVINGS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&previous_widths),
        )
        .expect("v3 column widths should be stored");

        let mut expected = DEFAULT_COLUMN_WIDTHS;
        expected[COL_TITLE] = 245.0;
        assert_eq!(
            parse_v3_column_widths(&format_column_widths(&previous_widths)),
            Some(expected.to_vec())
        );
        assert_eq!(load_column_widths(&database), expected);
        assert_eq!(
            finance_db::get_app_setting(&database, SAVINGS_COLUMN_WIDTHS_KEY)
                .expect("v4 setting should be readable"),
            None
        );
    }

    #[test]
    fn previous_savings_column_widths_follow_swapped_title_and_bank() {
        let database = filtered_database();
        let mut previous_widths = DEFAULT_V3_COLUMN_WIDTHS;
        // V2 хранил ширину банка первым, а названия — вторым.
        previous_widths.swap(COL_TITLE, COL_BANK);
        previous_widths[0] = 245.0;
        previous_widths[11] = 235.0;
        finance_db::set_app_setting(
            &database,
            OLDER_SAVINGS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&previous_widths),
        )
        .expect("previous column widths should be stored");

        let mut expected = DEFAULT_COLUMN_WIDTHS;
        expected[COL_BANK] = 245.0;
        expected[COL_ACTIONS] = ACTIONS_COLUMN_WIDTH;
        assert_eq!(
            parse_previous_column_widths(&format_column_widths(&previous_widths)),
            Some(expected.to_vec())
        );
        assert_eq!(load_column_widths(&database), expected);
    }

    #[test]
    fn legacy_savings_column_widths_are_remapped_to_the_new_column_order() {
        let database = filtered_database();
        let mut legacy_widths = DEFAULT_V3_COLUMN_WIDTHS;
        legacy_widths.swap(COL_TITLE, COL_BANK);
        legacy_widths[COL_OPENED_DATE] = 125.0;
        legacy_widths[COL_TERM] = 235.0;
        legacy_widths[11] = 220.0;
        finance_db::set_app_setting(
            &database,
            LEGACY_SAVINGS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&legacy_widths),
        )
        .expect("legacy column widths should be stored");

        let mut expected = DEFAULT_COLUMN_WIDTHS;
        expected[COL_OPENED_DATE] = 235.0;
        expected[COL_TERM] = 125.0;
        assert_eq!(
            parse_legacy_column_widths(&format_column_widths(&legacy_widths)),
            Some(expected.to_vec())
        );
        assert_eq!(load_column_widths(&database), expected);
    }

    #[test]
    fn changing_savings_column_widths_keeps_actions_fixed_and_other_widths() {
        let mut widths = DEFAULT_COLUMN_WIDTHS.to_vec();
        widths[COL_TITLE] = 275.0;
        widths[COL_INTEREST_ACTION] = 100.0;
        widths[COL_ACTIONS] = 230.0;

        let widths = column_widths_with_fixed_actions(widths);

        assert_eq!(widths[COL_TITLE], 275.0);
        assert_eq!(widths[COL_INTEREST_ACTION], INTEREST_ACTION_COLUMN_WIDTH);
        assert_eq!(widths[COL_ACTIONS], ACTIONS_COLUMN_WIDTH);
    }

    #[test]
    fn invalid_savings_column_widths_are_rejected() {
        assert!(parse_column_widths("60,not-a-width").is_none());
        assert!(
            parse_column_widths(&[59.0; 13].map(|width| width.to_string()).join(",")).is_none()
        );
        assert!(parse_column_widths(&format_column_widths(&DEFAULT_COLUMN_WIDTHS)).is_some());
    }

    fn filtered_database() -> finance_db::Database {
        let database =
            finance_db::Database::open_in_memory().expect("in-memory SQLite should open");
        for sql in [
            include_str!("../../../db/migrations/V1__initial.sql"),
            include_str!("../../../db/migrations/V2__transactions_and_directories.sql"),
            include_str!("../../../db/migrations/V3__savings.sql"),
            include_str!("../../../db/migrations/V4__payment_documents.sql"),
            include_str!("../../../db/migrations/V5__savings_dates.sql"),
            include_str!("../../../db/migrations/V6__income_card_balances.sql"),
            include_str!("../../../db/migrations/V7__expense_cards.sql"),
            include_str!("../../../db/migrations/V8__currencies.sql"),
            include_str!("../../../db/migrations/V9__savings_currency.sql"),
            include_str!("../../../db/migrations/V10__expense_card_balances.sql"),
            include_str!("../../../db/migrations/V11__expense_places_and_card_names.sql"),
            include_str!("../../../db/migrations/V12__transfers.sql"),
            include_str!("../../../db/migrations/V13__payment_document_next_payment.sql"),
            include_str!("../../../db/migrations/V14__prices.sql"),
            include_str!("../../../db/migrations/V15__other_interest_destination.sql"),
        ] {
            database.execute_batch(sql).expect("migration should apply");
        }
        database
            .execute_batch(
                "INSERT INTO banks (id, name) VALUES (1, 'Банк Альфа'), (2, 'Банк Бета');
                 INSERT INTO savings (
                     id, bank_id, title, amount, base_rate, term_type,
                     term_end_date, opened_date, interest_accrual_date,
                     interest_destination, is_active, currency
                 ) VALUES
                     (1, 1, 'Alpha', '100', '1', 'unlimited', NULL, '2026-03-01', '2026-03-31', 'reinvest', 1, 'USD'),
                     (2, 1, 'Alpha', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 0, NULL),
                     (3, 1, 'Gamma', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 1, 'RUB'),
                     (4, 2, 'Alpha', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 1, NULL),
                     (5, 2, 'Beta', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 0, NULL),
                     (6, 1, 'Другой вариант', '100', '1', 'unlimited', NULL, NULL, NULL, 'other', 1, 'RUB');",
            )
            .expect("savings fixtures should be inserted");
        database
    }

    fn ids(
        database: &finance_db::Database,
        bank: Option<i64>,
        active: Option<bool>,
        currency: Option<Currency>,
        search: &str,
    ) -> Vec<i64> {
        finance_db::list_savings_filtered(database, bank, active, currency, search)
            .expect("savings should load")
            .iter()
            .map(|entry| entry.id)
            .collect()
    }

    #[test]
    fn savings_filters_combine_and_clear_independently() {
        let database = filtered_database();
        assert_eq!(ids(&database, Some(1), Some(true), None, "Alpha"), [1]);
        assert_eq!(ids(&database, Some(1), Some(true), None, ""), [1, 3, 6]);
        assert_eq!(ids(&database, None, Some(true), None, "Alpha"), [1, 4]);
        assert_eq!(ids(&database, Some(1), None, None, "Alpha"), [1, 2]);
        assert_eq!(ids(&database, None, None, Some(Currency::Usd), ""), [1]);
        assert_eq!(ids(&database, None, None, None, "").len(), 6);
    }

    #[test]
    fn editing_form_roundtrips_existing_record_and_saves_changes() {
        let database = filtered_database();
        let entries = finance_db::list_savings_filtered(&database, None, None, None, "")
            .expect("savings should load");
        let account = entries.iter().find(|e| e.id == 1).expect("record 1");
        let mut form = SavingsForm::from_account(account);
        assert_eq!(form.id, Some(1));
        assert_eq!(form.bank_id, Some(1));
        assert_eq!(form.amount, "100");
        assert_eq!(form.currency, Some(Currency::Usd));
        assert_eq!(form.opened_date, "01.03.2026");
        assert_eq!(form.interest_accrual_date, "31.03.2026");

        form.title = "Изменённый вклад".to_owned();
        form.amount = "1250.50".to_owned();
        form.currency = Some(Currency::Rub);
        form.term_type = Some(SavingsTerm::Limited);
        form.term_end_date = "01.03.2027".to_owned();
        let details = form.details().expect("form should be valid");
        finance_db::save_savings(&database, form.id, &details).expect("save should succeed");

        let updated = finance_db::list_savings(&database, None, None, "")
            .expect("savings should load")
            .into_iter()
            .find(|e| e.id == 1)
            .expect("edit should preserve id");
        assert_eq!(updated.title, "Изменённый вклад");
        assert_eq!(updated.amount.to_string(), "1250.5");
        assert_eq!(updated.currency, Some(Currency::Rub));
        assert_eq!(updated.term_end_date, NaiveDate::from_ymd_opt(2027, 3, 1));
    }

    #[test]
    fn saving_with_unknown_bank_fails_without_changing_data() {
        let database = filtered_database();
        let mut form = valid_savings_form();
        form.id = Some(1);
        form.bank_id = Some(999);
        let details = form.details().expect("form itself is valid");
        assert!(finance_db::save_savings(&database, form.id, &details).is_err());
        let entry = finance_db::list_savings(&database, None, None, "")
            .expect("savings should load")
            .into_iter()
            .find(|e| e.id == 1)
            .expect("record 1");
        assert_eq!(entry.title, "Alpha");
    }
}
