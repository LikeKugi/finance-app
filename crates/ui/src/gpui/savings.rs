//! Страница «Накопления» на `gpui-kit`. Поведение соответствует
//! `savings_page.rs`; SQL и финансовые правила остаются в `finance-db` и
//! `finance-core`.

use std::cmp::Ordering;
use std::rc::Rc;

use chrono::NaiveDate;
use finance_core::{
    canonical_amount, parse_amount, parse_non_negative_decimal, Bank, Currency,
    InterestDestination, SavingsAccount, SavingsDetails, SavingsTerm,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::table::{
    Column, ColumnFixed, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::tooltip::Tooltip as ComponentTooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::StatefulInteractiveElement;
use gpui_kit::*;

use super::helpers::format_date;
use super::helpers::DATE_FORMAT;

const ALL_LABEL: &str = "Все";
const ALL_BANKS_LABEL: &str = "Все банки";
const NO_CURRENCY_LABEL: &str = "Не указана";
const TERMS: [SavingsTerm; 2] = [SavingsTerm::Limited, SavingsTerm::Unlimited];
const COLUMN_HEADINGS: [&str; 13] = [
    "Банк",
    "Название",
    "Сумма",
    "Валюта",
    "Базовая ставка",
    "Ставка при выполнении условий",
    "Порог",
    "Дата открытия",
    "Срок",
    "Дата начисления процентов",
    "Назначение процентов",
    "Статус",
    "Действия",
];
const DEFAULT_COLUMN_WIDTHS: [f32; 13] = [
    85.0, 120.0, 90.0, 70.0, 80.0, 170.0, 90.0, 95.0, 115.0, 105.0, 120.0, 85.0, 160.0,
];
const ACTIONS_COLUMN_WIDTH: f32 = 160.0;
const MIN_COLUMN_WIDTH: f32 = 60.0;
const MAX_COLUMN_WIDTH: f32 = 1200.0;
const SAVINGS_COLUMN_WIDTHS_KEY: &str = "table_columns.savings.v2";
const LEGACY_SAVINGS_COLUMN_WIDTHS_KEY: &str = "table_columns.savings";
const COL_BANK: usize = 0;
const COL_TITLE: usize = 1;
const COL_AMOUNT: usize = 2;
const COL_BASE_RATE: usize = 4;
const COL_RATE_AFTER_THRESHOLD: usize = 5;
const COL_THRESHOLD: usize = 6;
const COL_OPENED_DATE: usize = 7;
const COL_TERM: usize = 8;
const COL_ACTIONS: usize = 12;
const FOOTER_NOTE: &str = "Даты открытия и начисления процентов сохраняются как справочные значения; процентный доход здесь не рассчитывается без истории остатка и правил начисления банка. Повышенная ставка показана как условие, а не как подтверждённая доходность.";
const FOOTER_WARNING: &str =
    "Валюта накопления указывается отдельно; фактические начисления процентов здесь не рассчитываются.";

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
        subscriptions.push(
            cx.subscribe(&table, |this: &mut Self, _, event: &TableEvent, cx| {
                if let TableEvent::ColumnWidthsChanged(widths) = event {
                    this.save_column_widths(widths, cx);
                }
            }),
        );

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
            panel = panel.child(div().text_color(danger).child(error));
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
    let widths = value
        .split(',')
        .map(|part| part.trim().parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()?;
    (widths.len() == COLUMN_HEADINGS.len()
        && widths.iter().all(|width| {
            width.is_finite() && (MIN_COLUMN_WIDTH..=MAX_COLUMN_WIDTH).contains(width)
        }))
    .then_some(widths)
}

fn parse_legacy_column_widths(value: &str) -> Option<Vec<f32>> {
    let mut widths = parse_column_widths(value)?;
    widths.swap(COL_OPENED_DATE, COL_TERM);
    Some(column_widths_with_fixed_actions(widths))
}

fn column_widths_with_fixed_actions(mut widths: Vec<f32>) -> Vec<f32> {
    if let Some(width) = widths.get_mut(COL_ACTIONS) {
        *width = ACTIONS_COLUMN_WIDTH;
    }
    widths
}

fn load_column_widths(database: &Database) -> Vec<f32> {
    match finance_db::get_app_setting(database, column_widths_key()) {
        Ok(Some(value)) => parse_column_widths(&value)
            .map(column_widths_with_fixed_actions)
            .unwrap_or_else(|| {
                eprintln!("Некорректная сохранённая ширина колонок накоплений; использованы значения по умолчанию");
                DEFAULT_COLUMN_WIDTHS.to_vec()
            }),
        Ok(None) => match finance_db::get_app_setting(database, LEGACY_SAVINGS_COLUMN_WIDTHS_KEY) {
            Ok(Some(value)) => parse_legacy_column_widths(&value).unwrap_or_else(|| {
                eprintln!(
                    "Некорректная сохранённая ширина колонок накоплений; использованы значения по умолчанию"
                );
                DEFAULT_COLUMN_WIDTHS.to_vec()
            }),
            Ok(None) => DEFAULT_COLUMN_WIDTHS.to_vec(),
            Err(error) => {
                eprintln!("Не удалось прочитать ширину колонок накоплений: {error}");
                DEFAULT_COLUMN_WIDTHS.to_vec()
            }
        },
        Err(error) => {
            eprintln!("Не удалось прочитать ширину колонок накоплений: {error}");
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
        let column = Column::new(COLUMN_HEADINGS[col_ix], COLUMN_HEADINGS[col_ix])
            .width(px(self.widths[col_ix]))
            .min_width(px(if is_actions {
                ACTIONS_COLUMN_WIDTH
            } else {
                MIN_COLUMN_WIDTH
            }))
            .max_width(px(if is_actions {
                ACTIONS_COLUMN_WIDTH
            } else {
                MAX_COLUMN_WIDTH
            }))
            .resizable(!is_actions)
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
            | COL_TERM => column.sort(direction),
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
            .child("Накоплений по выбранным условиям нет.")
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
            COL_BANK => div().child(account.bank_name.clone()).into_any_element(),
            COL_TITLE => div().child(account.title.clone()).into_any_element(),
            COL_AMOUNT => div()
                .w_full()
                .text_right()
                .child(account.amount.to_string())
                .into_any_element(),
            3 => div()
                .child(
                    account
                        .currency
                        .map_or_else(dash, |currency| currency.code().to_owned()),
                )
                .into_any_element(),
            COL_BASE_RATE => div()
                .child(format!("{}%", account.base_rate))
                .into_any_element(),
            COL_RATE_AFTER_THRESHOLD => div()
                .child(
                    account
                        .rate_after_threshold
                        .map_or_else(dash, |rate| format!("{rate}%")),
                )
                .into_any_element(),
            COL_THRESHOLD => div()
                .child(
                    account
                        .threshold_amount
                        .map_or_else(dash, |amount| amount.to_string()),
                )
                .into_any_element(),
            COL_TERM => div().child(term).into_any_element(),
            COL_OPENED_DATE => div()
                .child(account.opened_date.map_or_else(dash, format_date))
                .into_any_element(),
            9 => div()
                .child(account.interest_accrual_date.map_or_else(dash, format_date))
                .into_any_element(),
            10 => div()
                .child(account.interest_destination.title().to_owned())
                .into_any_element(),
            11 => div()
                .child(if account.is_active {
                    "Активно"
                } else {
                    "Неактивно"
                })
                .into_any_element(),
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
            root = root
                .child("Сначала добавьте банк в разделе «Справочники», чтобы создать накопление.");
        }
        if let Some((message, is_error)) = &self.message {
            root = root.child(
                div()
                    .text_color(if *is_error { danger } else { success })
                    .child(message.clone()),
            );
        }
        root = root.child(self.render_table(cx));
        root.child(div().text_color(muted).child(FOOTER_NOTE))
            .child(div().text_color(warning).child(FOOTER_WARNING))
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use finance_core::{Currency, InterestDestination, SavingsAccount, SavingsTerm};
    use gpui_kit::component::table::ColumnSort;
    use rust_decimal::Decimal;

    use super::{
        column_widths_key, column_widths_with_fixed_actions, destination_filter_labels,
        filter_savings, format_column_widths, load_column_widths, parse_column_widths,
        parse_legacy_column_widths, sort_savings, term_filter_labels, tooltip_items, SavingsForm,
        ACTIONS_COLUMN_WIDTH, BANK_FILTER_ITEM_ID, COLUMN_HEADINGS, COL_ACTIONS, COL_AMOUNT,
        COL_BANK, COL_BASE_RATE, COL_OPENED_DATE, COL_RATE_AFTER_THRESHOLD, COL_TERM,
        COL_THRESHOLD, COL_TITLE, DEFAULT_COLUMN_WIDTHS, LEGACY_SAVINGS_COLUMN_WIDTHS_KEY,
    };

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
        let entries = sortable_savings();
        let ids =
            |rows: Vec<SavingsAccount>| rows.into_iter().map(|row| row.id).collect::<Vec<_>>();

        assert_eq!(
            term_filter_labels(),
            ["Все", "С ограниченным сроком", "Бессрочный"]
        );
        assert_eq!(
            destination_filter_labels(),
            ["Все", "Капитализация", "На карту"]
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
            [2]
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
                Some(SavingsTerm::Limited),
                Some(InterestDestination::Withdraw)
            )),
            [3]
        );
        assert_eq!(ids(filter_savings(entries.clone(), None, None)), [1, 2, 3]);
    }

    #[test]
    fn savings_column_widths_are_loaded_from_database_and_validated() {
        let database = filtered_database();
        let mut widths = DEFAULT_COLUMN_WIDTHS;
        widths[1] = 245.0;
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
    fn legacy_savings_column_widths_are_remapped_to_the_new_column_order() {
        let database = filtered_database();
        let mut legacy_widths = DEFAULT_COLUMN_WIDTHS;
        legacy_widths[COL_OPENED_DATE] = 125.0;
        legacy_widths[COL_TERM] = 235.0;
        legacy_widths[COL_ACTIONS] = 220.0;
        finance_db::set_app_setting(
            &database,
            LEGACY_SAVINGS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&legacy_widths),
        )
        .expect("legacy column widths should be stored");

        let mut expected = legacy_widths;
        expected.swap(COL_OPENED_DATE, COL_TERM);
        expected[COL_ACTIONS] = ACTIONS_COLUMN_WIDTH;
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
        widths[COL_ACTIONS] = 230.0;

        let widths = column_widths_with_fixed_actions(widths);

        assert_eq!(widths[COL_TITLE], 275.0);
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
                     (5, 2, 'Beta', '100', '1', 'unlimited', NULL, NULL, NULL, 'reinvest', 0, NULL);",
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
        assert_eq!(ids(&database, Some(1), Some(true), None, ""), [1, 3]);
        assert_eq!(ids(&database, None, Some(true), None, "Alpha"), [1, 4]);
        assert_eq!(ids(&database, Some(1), None, None, "Alpha"), [1, 2]);
        assert_eq!(ids(&database, None, None, Some(Currency::Usd), ""), [1]);
        assert_eq!(ids(&database, None, None, None, "").len(), 5);
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
