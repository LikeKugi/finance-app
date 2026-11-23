//! Страницы «Доходы» и «Расходы» на `gpui-kit`. Поведение соответствует
//! `transactions_page.rs`; SQL и финансовые правила остаются в `finance-db` и
//! `finance-core`.

use std::collections::HashSet;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use finance_core::{
    parse_amount_expression, summarize_transaction_balance, Bank, Category, Currency,
    CurrencyDefinition, ExpensePlace, FinancialTransaction, PaymentCardType, PaymentDocument,
    PaymentDocumentFilter, TransactionBalance, TransactionKind,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
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
use rust_decimal::Decimal;

use super::helpers::{format_currency, format_date};
use super::helpers::DATE_FORMAT;
use super::support::{
    card_label, category_tree_labels, collect_category_descendants, eligible_cards,
    expense_place_label, expense_place_tooltip, text_suggestions, DIVISION_ROUNDING_WARNING,
};
use super::support::{date_period_range, DatePeriod, DatePeriodRange};

const NONE_LABEL: &str = "Все";
const NO_CARD_LABEL: &str = "Не по карте";
const EXPENSE_PLACE_LABEL: &str = "Место покупки";
const EXPENSE_PLACE_FILTER_ITEM_ID: &str = "expense-place-filter-item";
const INCOME_PAYMENT_FILTER_LABEL: &str = "Способ получения";
const PAYMENT_FILTER_LABEL: &str = "Способ оплаты";

type Choice = Entity<SelectState<Vec<SharedString>>>;
type CategoryChoice = Entity<SelectState<Vec<CategoryFilterItem>>>;
type BankChoice = Entity<SelectState<Vec<BankFilterItem>>>;
type CardChoice = Entity<SelectState<Vec<CardFilterItem>>>;
type PlaceChoice = Entity<SelectState<Vec<PlaceFilterItem>>>;

#[derive(Clone)]
struct CategoryFilterItem {
    id: usize,
    label: String,
}

impl SelectItem for CategoryFilterItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.label
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip = self.label.clone();
        div()
            .id(("category-filter-item", self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}

#[derive(Clone)]
struct BankFilterItem {
    id: usize,
    label: String,
}

impl SelectItem for BankFilterItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.label
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip = self.label.clone();
        div()
            .id(("bank-filter-item", self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}

#[derive(Clone)]
struct CardFilterItem {
    id: usize,
    label: String,
}

impl SelectItem for CardFilterItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.label
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip = self.label.clone();
        div()
            .id(("card-filter-item", self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}

#[derive(Clone)]
struct PlaceFilterItem {
    id: usize,
    label: String,
    tooltip: Option<String>,
}

impl SelectItem for PlaceFilterItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.label
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id((EXPENSE_PLACE_FILTER_ITEM_ID, self.id))
            .w_full()
            .when_some(self.tooltip.clone(), |element, tooltip| {
                element.tooltip(move |window, cx| {
                    ComponentTooltip::new(tooltip.clone()).build(window, cx)
                })
            })
            .child(self.label.clone())
    }
}

fn choice(
    items: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<TransactionsPage>,
) -> Choice {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn category_choice(
    labels: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<TransactionsPage>,
) -> CategoryChoice {
    let items: Vec<CategoryFilterItem> = labels
        .into_iter()
        .enumerate()
        .map(|(id, label)| CategoryFilterItem { id, label })
        .collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn bank_choice(
    labels: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<TransactionsPage>,
) -> BankChoice {
    let items: Vec<BankFilterItem> = labels
        .into_iter()
        .enumerate()
        .map(|(id, label)| BankFilterItem { id, label })
        .collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn card_choice(
    labels: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<TransactionsPage>,
) -> CardChoice {
    let items: Vec<CardFilterItem> = labels
        .into_iter()
        .enumerate()
        .map(|(id, label)| CardFilterItem { id, label })
        .collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn place_choice(
    labels: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<TransactionsPage>,
) -> PlaceChoice {
    let items: Vec<PlaceFilterItem> = labels
        .into_iter()
        .enumerate()
        .map(|(id, label)| PlaceFilterItem {
            id,
            label,
            tooltip: None,
        })
        .collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn selected_date(state: &DatePickerState) -> Option<NaiveDate> {
    match state.date() {
        Date::Single(date) => date,
        Date::Range(_, _) => None,
    }
}

fn period_dates(
    period: DatePeriod,
    today: NaiveDate,
) -> Option<(Option<NaiveDate>, Option<NaiveDate>)> {
    date_period_range(period, today).map(|range| match range {
        DatePeriodRange::Unbounded => (None, None),
        DatePeriodRange::Bounded(from, to) => (Some(from), Some(to)),
    })
}

fn cards_for_bank(cards: &[PaymentDocument], bank_id: Option<i64>) -> Vec<&PaymentDocument> {
    cards
        .iter()
        .filter(|card| bank_id.is_none_or(|bank_id| card.bank_id == bank_id))
        .collect()
}

fn update_bank_filter_selection(
    bank_filter: &mut Option<i64>,
    card_filter: &mut PaymentDocumentFilter,
    next_bank: Option<i64>,
) -> bool {
    if *bank_filter == next_bank {
        return false;
    }
    *bank_filter = next_bank;
    *card_filter = PaymentDocumentFilter::All;
    true
}

/// Индекс 0 — «Все», индекс 1 — «Не по карте», остальные — карты банка.
fn card_filter_from_index(index: usize, card_ids: &[i64]) -> PaymentDocumentFilter {
    match index {
        0 => PaymentDocumentFilter::All,
        1 => PaymentDocumentFilter::WithoutCard,
        index => card_ids
            .get(index - 2)
            .map_or(PaymentDocumentFilter::All, |id| {
                PaymentDocumentFilter::Card(*id)
            }),
    }
}

fn card_filter_index(filter: PaymentDocumentFilter, card_ids: &[i64]) -> Option<usize> {
    match filter {
        PaymentDocumentFilter::All => Some(0),
        PaymentDocumentFilter::WithoutCard => Some(1),
        PaymentDocumentFilter::Card(id) => card_ids
            .iter()
            .position(|candidate| *candidate == id)
            .map(|position| position + 2),
    }
}

fn payment_filter_labels(cards: &[PaymentDocument], bank_filter: Option<i64>) -> Vec<String> {
    std::iter::once(NONE_LABEL.to_owned())
        .chain(std::iter::once(NO_CARD_LABEL.to_owned()))
        .chain(
            cards_for_bank(cards, bank_filter)
                .into_iter()
                .map(card_label),
        )
        .collect()
}

/// Подпись фильтра карты зависит от операции: доход получают, расход оплачивают.
fn payment_filter_label(kind: TransactionKind) -> &'static str {
    match kind {
        TransactionKind::Income => INCOME_PAYMENT_FILTER_LABEL,
        TransactionKind::Expense => PAYMENT_FILTER_LABEL,
    }
}

struct Form {
    id: Option<i64>,
    date: Entity<DatePickerState>,
    amount: Entity<InputState>,
    comment: Entity<TextareaState>,
    currency: Option<Currency>,
    currency_select: Choice,
    category_ids: Vec<i64>,
    category_id: Option<i64>,
    category_select: Choice,
    expense_place_ids: Vec<Option<i64>>,
    expense_place_id: Option<i64>,
    expense_place_select: Choice,
    card_ids: Vec<Option<i64>>,
    card_id: Option<i64>,
    card_select: Choice,
    affects_deposit_rate: bool,
    balance_applied: bool,
    suggestions: Vec<String>,
    initial_values: FormValues,
}

#[derive(Debug, PartialEq)]
struct FormValues {
    date: Option<NaiveDate>,
    amount: String,
    comment: String,
    currency: Option<Currency>,
    category_id: Option<i64>,
    expense_place_id: Option<i64>,
    card_id: Option<i64>,
    affects_deposit_rate: bool,
}

impl FormValues {
    fn changed_from(&self, initial: &Self) -> bool {
        self != initial
    }
}

pub(crate) struct TransactionsPage {
    database: Rc<Database>,
    kind: TransactionKind,
    categories: Vec<Category>,
    expense_places: Vec<ExpensePlace>,
    banks: Vec<Bank>,
    currencies: Vec<CurrencyDefinition>,
    cards: Vec<PaymentDocument>,
    filter_cards: Vec<PaymentDocument>,
    comment_history: Vec<String>,
    transactions: Vec<FinancialTransaction>,
    table: Entity<TableState<TransactionsTable>>,
    balances: Option<[TransactionBalance; 2]>,
    balance_period: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
    message: Option<(String, bool)>,
    search: Entity<InputState>,
    from: Entity<DatePickerState>,
    to: Entity<DatePickerState>,
    category_filter: Option<i64>,
    category_filter_ids: Vec<i64>,
    category_filter_select: CategoryChoice,
    expense_place_filter: Option<i64>,
    expense_place_filter_ids: Vec<i64>,
    expense_place_filter_select: PlaceChoice,
    bank_filter: Option<i64>,
    bank_filter_ids: Vec<i64>,
    bank_filter_select: BankChoice,
    card_filter: PaymentDocumentFilter,
    card_filter_ids: Vec<i64>,
    card_filter_select: CardChoice,
    currency_filter: Option<Currency>,
    currency_filter_select: Choice,
    period_select: Choice,
    form: Option<Form>,
    confirm_discard: bool,
    _subscriptions: Vec<Subscription>,
}

impl TransactionsPage {
    pub(crate) fn new(
        database: Rc<Database>,
        kind: TransactionKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Поиск"));
        let from = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let to = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let category_filter_select = category_choice(vec![NONE_LABEL.to_owned()], 0, window, cx);
        let expense_place_filter_select = place_choice(vec![NONE_LABEL.to_owned()], 0, window, cx);
        let bank_filter_select = bank_choice(vec![NONE_LABEL.to_owned()], 0, window, cx);
        let card_filter_select = card_choice(
            vec![NONE_LABEL.to_owned(), NO_CARD_LABEL.to_owned()],
            0,
            window,
            cx,
        );
        let currency_filter_select = choice(Self::currency_filter_labels(), 0, window, cx);
        let period_select = choice(Self::period_labels(), 0, window, cx);

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.refresh(cx);
                }
            },
        ));
        for date_picker in [&from, &to] {
            subscriptions.push(cx.subscribe_in(
                date_picker,
                window,
                |_, _, _: &DatePickerEvent, _, cx| cx.notify(),
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &category_filter_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<CategoryFilterItem>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.category_filter = value
                    .as_deref()
                    .and_then(|label| {
                        this.category_filter_labels()
                            .iter()
                            .position(|l| l == label)
                    })
                    .and_then(|position| position.checked_sub(1))
                    .and_then(|position| this.category_filter_ids.get(position).copied());
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &expense_place_filter_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<PlaceFilterItem>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.expense_place_filter = value
                    .as_ref()
                    .and_then(|label| {
                        this.expense_place_filter_labels()
                            .iter()
                            .position(|item| item == label)
                    })
                    .and_then(|position| position.checked_sub(1))
                    .and_then(|position| this.expense_place_filter_ids.get(position).copied());
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &bank_filter_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<BankFilterItem>>, window, cx| {
                let SelectEvent::Confirm(value) = event;
                let next_bank = value
                    .as_deref()
                    .and_then(|label| {
                        this.bank_filter_labels()
                            .iter()
                            .position(|item| item.as_str() == label)
                    })
                    .and_then(|position| position.checked_sub(1))
                    .and_then(|position| this.bank_filter_ids.get(position).copied());
                update_bank_filter_selection(
                    &mut this.bank_filter,
                    &mut this.card_filter,
                    next_bank,
                );
                this.sync_card_filter_options(window, cx);
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &card_filter_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<CardFilterItem>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                let selected_index = value.as_deref().and_then(|label| {
                    this.card_filter_labels()
                        .iter()
                        .position(|item| item.as_str() == label)
                });
                this.card_filter = selected_index.map_or(PaymentDocumentFilter::All, |index| {
                    card_filter_from_index(index, &this.card_filter_ids)
                });
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &currency_filter_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.currency_filter = value.as_ref().and_then(|label| {
                    Currency::ALL
                        .into_iter()
                        .find(|currency| currency.code() == label.as_ref())
                });
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &period_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                if let Some((period, _)) = DatePeriod::ALL
                    .into_iter()
                    .find(|(_, name)| *name == label.as_ref())
                {
                    this.apply_period(period, window, cx);
                }
            },
        ));

        let widths = load_column_widths(&database, kind);
        let table_delegate = TransactionsTable::new(cx.entity().downgrade(), kind, widths);
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
            kind,
            categories: Vec::new(),
            expense_places: Vec::new(),
            banks: Vec::new(),
            currencies: Vec::new(),
            cards: Vec::new(),
            filter_cards: Vec::new(),
            comment_history: Vec::new(),
            transactions: Vec::new(),
            table,
            balances: None,
            balance_period: None,
            message: None,
            search,
            from,
            to,
            category_filter: None,
            category_filter_ids: Vec::new(),
            category_filter_select,
            expense_place_filter: None,
            expense_place_filter_ids: Vec::new(),
            expense_place_filter_select,
            bank_filter: None,
            bank_filter_ids: Vec::new(),
            bank_filter_select,
            card_filter: PaymentDocumentFilter::All,
            card_filter_ids: Vec::new(),
            card_filter_select,
            currency_filter: None,
            currency_filter_select,
            period_select,
            form: None,
            confirm_discard: false,
            _subscriptions: subscriptions,
        };
        page.reload_categories(window, cx);
        page.reload_filter_sources(window, cx);
        page.refresh(cx);
        page
    }

    /// Обновляет категории и список после изменений в справочниках.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.reload_filter_sources(window, cx) {
            return;
        }
        self.reload_categories(window, cx);
        self.refresh(cx);
    }

    fn currency_filter_labels() -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(Currency::ALL.into_iter().map(|c| c.code().to_owned()))
            .collect()
    }

    fn period_labels() -> Vec<String> {
        DatePeriod::ALL
            .into_iter()
            .map(|(_, name)| name.to_owned())
            .collect()
    }

    fn category_filter_labels(&self) -> Vec<String> {
        let labels = category_tree_labels(&self.categories);
        std::iter::once(NONE_LABEL.to_owned())
            .chain(labels.into_iter().map(|(_, label)| label))
            .collect()
    }

    fn expense_place_filter_labels(&self) -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(self.expense_places.iter().map(expense_place_label))
            .collect()
    }

    fn bank_filter_labels(&self) -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(self.banks.iter().map(|bank| bank.name.clone()))
            .collect()
    }

    fn card_filter_labels(&self) -> Vec<String> {
        payment_filter_labels(&self.filter_cards, self.bank_filter)
    }

    fn save_column_widths(&mut self, widths: &[Pixels], cx: &mut Context<Self>) {
        let widths = column_widths_with_fixed_actions(
            widths.iter().map(|width| f32::from(*width)).collect(),
            self.kind,
        );
        self.table
            .update(cx, |table, _| table.delegate_mut().set_widths(&widths));
        if let Err(error) = finance_db::set_app_setting(
            &self.database,
            column_widths_key(self.kind),
            &format_column_widths(&widths),
        ) {
            self.error(format!("Не удалось сохранить ширину колонок: {error}"), cx);
        }
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some((message.into(), true));
        cx.notify();
    }

    fn reload_categories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match finance_db::list_categories(&self.database, self.kind, true) {
            Ok(categories) => self.categories = categories,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        }
        self.category_filter_ids = category_tree_labels(&self.categories)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let labels = self.category_filter_labels();
        let items: Vec<CategoryFilterItem> = labels
            .into_iter()
            .enumerate()
            .map(|(id, label)| CategoryFilterItem { id, label })
            .collect();
        self.category_filter_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
        });
    }

    fn reload_filter_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let banks = match finance_db::list_banks(&self.database) {
            Ok(banks) => banks,
            Err(error) => {
                self.error(error.to_string(), cx);
                return false;
            }
        };
        let cards = match finance_db::list_payment_documents(&self.database, None, None, None) {
            Ok(cards) => cards,
            Err(error) => {
                self.error(error.to_string(), cx);
                return false;
            }
        };
        let expense_places = match finance_db::list_expense_places(&self.database) {
            Ok(places) => places,
            Err(error) => {
                self.error(error.to_string(), cx);
                return false;
            }
        };
        self.banks = banks;
        self.filter_cards = cards;
        self.expense_places = expense_places;
        if self
            .bank_filter
            .is_some_and(|bank_id| !self.banks.iter().any(|bank| bank.id == bank_id))
        {
            self.bank_filter = None;
            self.card_filter = PaymentDocumentFilter::All;
        }
        if let PaymentDocumentFilter::Card(card_id) = self.card_filter {
            if !cards_for_bank(&self.filter_cards, self.bank_filter)
                .iter()
                .any(|card| card.id == card_id)
            {
                self.card_filter = PaymentDocumentFilter::All;
            }
        }
        if self
            .expense_place_filter
            .is_some_and(|place_id| !self.expense_places.iter().any(|place| place.id == place_id))
        {
            self.expense_place_filter = None;
        }

        self.bank_filter_ids = self.banks.iter().map(|bank| bank.id).collect();
        self.expense_place_filter_ids = self.expense_places.iter().map(|place| place.id).collect();
        let selected_place_index = self
            .expense_place_filter
            .and_then(|place_id| {
                self.expense_place_filter_ids
                    .iter()
                    .position(|candidate| *candidate == place_id)
            })
            .map_or(0, |position| position + 1);
        let expense_place_items: Vec<PlaceFilterItem> = self
            .expense_place_filter_labels()
            .into_iter()
            .enumerate()
            .map(|(id, label)| PlaceFilterItem {
                id,
                tooltip: id
                    .checked_sub(1)
                    .and_then(|row| self.expense_places.get(row))
                    .map(expense_place_tooltip),
                label,
            })
            .collect();
        self.expense_place_filter_select.update(cx, |state, cx| {
            state.set_items(expense_place_items, window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(selected_place_index)),
                window,
                cx,
            );
        });
        let bank_items: Vec<BankFilterItem> = self
            .bank_filter_labels()
            .into_iter()
            .enumerate()
            .map(|(id, label)| BankFilterItem { id, label })
            .collect();
        let selected_bank_index = self
            .bank_filter
            .and_then(|bank_id| {
                self.bank_filter_ids
                    .iter()
                    .position(|candidate| *candidate == bank_id)
            })
            .map_or(0, |position| position + 1);
        self.bank_filter_select.update(cx, |state, cx| {
            state.set_items(bank_items, window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(selected_bank_index)),
                window,
                cx,
            );
        });
        self.sync_card_filter_options(window, cx);
        true
    }

    fn sync_card_filter_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let available_cards = cards_for_bank(&self.filter_cards, self.bank_filter);
        self.card_filter_ids = available_cards.iter().map(|card| card.id).collect();
        let selected_card_index = card_filter_index(self.card_filter, &self.card_filter_ids)
            .unwrap_or_else(|| {
                self.card_filter = PaymentDocumentFilter::All;
                0
            });
        let items: Vec<CardFilterItem> = self
            .card_filter_labels()
            .into_iter()
            .enumerate()
            .map(|(id, label)| CardFilterItem { id, label })
            .collect();
        self.card_filter_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(
                Some(IndexPath::default().row(selected_card_index)),
                window,
                cx,
            );
        });
    }

    /// Загружает операции и сальдо по текущим фильтрам; ошибки показываются, а не маскируются.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let from = selected_date(self.from.read(cx));
        let to = selected_date(self.to.read(cx));
        let search = self.search.read(cx).value().to_string();
        if matches!((from, to), (Some(start), Some(end)) if start > end) {
            self.error("Начало периода не может быть позже окончания", cx);
            return;
        }
        self.balances = None;
        self.balance_period = None;
        match finance_db::list_currencies(&self.database) {
            Ok(currencies) => self.currencies = currencies,
            Err(error) => {
                self.currencies.clear();
                self.error(error.to_string(), cx);
                return;
            }
        }
        let category_ids = self
            .category_filter
            .map(|id| collect_category_descendants(&self.categories, id));
        match finance_db::list_transactions_filtered_by_place(
            &self.database,
            self.kind,
            from,
            to,
            category_ids,
            self.currency_filter,
            &search,
            self.bank_filter,
            self.card_filter,
            self.expense_place_filter,
        ) {
            Ok(transactions) => {
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_rows(transactions.clone());
                    cx.notify();
                });
                self.transactions = transactions;
            }
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        }
        let balances = finance_db::list_transaction_amounts(&self.database, from, to)
            .map_err(|error| error.to_string())
            .and_then(|amounts| {
                summarize_transaction_balance(&amounts).map_err(|error| error.to_string())
            });
        match balances {
            Ok(balances) => {
                self.balances = Some(balances);
                self.balance_period = Some((from, to));
                self.message = None;
                cx.notify();
            }
            Err(message) => self.error(message, cx),
        }
    }

    fn apply_period(&mut self, period: DatePeriod, window: &mut Window, cx: &mut Context<Self>) {
        let Some((from, to)) = period_dates(period, Local::now().date_naive()) else {
            return;
        };
        self.from.update(cx, |state, cx| {
            state.set_date(Date::Single(from), window, cx)
        });
        self.to
            .update(cx, |state, cx| state.set_date(Date::Single(to), window, cx));
        self.refresh(cx);
    }

    fn has_active_filters(&self, cx: &App) -> bool {
        !self.search.read(cx).value().is_empty()
            || selected_date(self.from.read(cx)).is_some()
            || selected_date(self.to.read(cx)).is_some()
            || self.category_filter.is_some()
            || self.expense_place_filter.is_some()
            || self.bank_filter.is_some()
            || self.card_filter.is_active()
            || self.currency_filter.is_some()
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        for date_picker in [&self.from, &self.to] {
            date_picker.update(cx, |state, cx| {
                state.set_date(Date::Single(None), window, cx)
            });
        }
        self.category_filter = None;
        self.expense_place_filter = None;
        self.bank_filter = None;
        self.card_filter = PaymentDocumentFilter::All;
        self.currency_filter = None;
        self.sync_card_filter_options(window, cx);
        self.category_filter_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        self.expense_place_filter_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        for select in [&self.currency_filter_select, &self.period_select] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
            });
        }
        self.bank_filter_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        self.card_filter_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        self.refresh(cx);
    }

    fn primary_currency(&self) -> Option<&str> {
        self.currencies
            .iter()
            .find(|currency| currency.is_primary)
            .map(|currency| currency.code.as_str())
    }

    // ---- форма ----

    fn load_form_sources(&mut self, cx: &mut Context<Self>) -> bool {
        let cards = finance_db::list_payment_documents(
            &self.database,
            None,
            (self.kind == TransactionKind::Income).then_some(PaymentCardType::Debit),
            None,
        );
        match cards {
            Ok(cards) => self.cards = cards,
            Err(error) => {
                self.error(error.to_string(), cx);
                return false;
            }
        }
        match finance_db::list_transaction_comments(&self.database) {
            Ok(comments) => self.comment_history = comments,
            Err(error) => {
                self.error(error.to_string(), cx);
                return false;
            }
        }
        true
    }

    fn open_form(
        &mut self,
        existing: Option<&FinancialTransaction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.reload_filter_sources(window, cx) || !self.load_form_sources(cx) {
            return;
        }
        self.message = None;
        self.confirm_discard = false;
        self.reload_categories(window, cx);
        let currency = existing.map_or(Some(Currency::Rub), |t| Some(t.currency));
        let category_id = existing.map(|t| t.category_id).or_else(|| {
            self.categories
                .iter()
                .find(|category| category.is_active)
                .map(|category| category.id)
        });
        let card_id = existing.and_then(|t| t.payment_document_id);
        let expense_place_id = existing.and_then(|t| t.expense_place_id);
        let date =
            existing.map_or_else(|| Local::now().date_naive(), |transaction| transaction.date);
        let initial_values = FormValues {
            date: Some(date),
            amount: existing.map_or_else(String::new, |transaction| transaction.amount.to_string()),
            comment: existing.map_or_else(String::new, |transaction| transaction.comment.clone()),
            currency,
            category_id,
            expense_place_id,
            card_id,
            affects_deposit_rate: existing.is_some_and(|t| t.affects_deposit_rate),
        };

        let form_date = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        form_date.update(cx, |state, cx| {
            state.set_date(Date::Single(Some(date)), window, cx)
        });
        self._subscriptions.push(cx.subscribe_in(
            &form_date,
            window,
            |_, _, _: &DatePickerEvent, _, cx| cx.notify(),
        ));
        let amount_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder("например: 100 + 20 - 5 * 2 / 4");
            if let Some(transaction) = existing {
                state.set_value(transaction.amount.to_string(), window, cx);
            }
            state
        });
        let comment_input = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx).rows(3);
            if let Some(transaction) = existing {
                state.set_value(transaction.comment.clone(), window, cx);
            }
            state
        });
        let currency_select = choice(
            Currency::ALL.iter().map(|c| c.code().to_owned()).collect(),
            Currency::ALL
                .iter()
                .position(|c| Some(*c) == currency)
                .unwrap_or(0),
            window,
            cx,
        );
        let category_select = choice(Vec::new(), 0, window, cx);
        let card_select = choice(Vec::new(), 0, window, cx);
        let expense_place_ids: Vec<Option<i64>> = std::iter::once(None)
            .chain(self.expense_places.iter().map(|place| Some(place.id)))
            .collect();
        let expense_place_items = std::iter::once("Не указано".to_owned())
            .chain(self.expense_places.iter().map(expense_place_label))
            .collect();
        let selected_place_index = expense_place_ids
            .iter()
            .position(|id| *id == expense_place_id)
            .unwrap_or(0);
        let expense_place_select = choice(expense_place_items, selected_place_index, window, cx);

        let subs = [
            cx.subscribe_in(&amount_input, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &comment_input,
                window,
                |this: &mut Self, state, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = state.read(cx).value().to_string();
                        let suggestions = text_suggestions(&this.comment_history, &text);
                        if let Some(form) = this.form.as_mut() {
                            form.suggestions = suggestions;
                        }
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &currency_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let currency = value.as_ref().and_then(|label| {
                        Currency::ALL
                            .into_iter()
                            .find(|c| c.code() == label.as_ref())
                    });
                    if let Some(form) = this.form.as_mut() {
                        form.currency = currency;
                        form.card_id = None;
                    }
                    this.rebuild_card_select(window, cx);
                },
            ),
            cx.subscribe_in(
                &category_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let (Some(form), Some(label)) = (this.form.as_mut(), value) {
                        let labels = category_tree_labels(&this.categories);
                        form.category_id = labels
                            .iter()
                            .find(|(id, l)| l == label.as_ref() && form.category_ids.contains(id))
                            .map(|(id, _)| *id);
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &expense_place_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let (Some(form), Some(label)) = (this.form.as_mut(), value) {
                        let labels: Vec<String> = std::iter::once("Не указано".to_owned())
                            .chain(this.expense_places.iter().map(expense_place_label))
                            .collect();
                        form.expense_place_id = labels
                            .iter()
                            .position(|item| item == label.as_ref())
                            .and_then(|position| form.expense_place_ids.get(position).copied())
                            .flatten();
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &card_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let (Some(form), Some(label)) = (this.form.as_mut(), value) {
                        let cards = eligible_cards(&this.cards, form.currency);
                        form.card_id = cards
                            .iter()
                            .find(|card| card_label(card) == label.as_ref())
                            .map(|card| card.id);
                        let eligible = cards
                            .iter()
                            .any(|card| Some(card.id) == form.card_id && card.affects_deposit_rate);
                        if !eligible {
                            form.affects_deposit_rate = false;
                        }
                    }
                    cx.notify();
                },
            ),
        ];
        self._subscriptions.extend(subs);

        self.form = Some(Form {
            id: existing.map(|t| t.id),
            date: form_date,
            amount: amount_input,
            comment: comment_input,
            currency,
            currency_select,
            category_ids: Vec::new(),
            category_id,
            category_select,
            expense_place_ids,
            expense_place_id,
            expense_place_select,
            card_ids: Vec::new(),
            card_id,
            card_select,
            affects_deposit_rate: existing.is_some_and(|t| t.affects_deposit_rate),
            balance_applied: existing.is_some_and(|t| t.payment_document_balance_applied),
            suggestions: Vec::new(),
            initial_values,
        });
        self.rebuild_category_select(window, cx);
        self.rebuild_card_select(window, cx);
        cx.notify();
        self.open_form_dialog(window, cx);
    }

    fn open_form_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        window.open_dialog(cx, move |dialog: Dialog, _, cx| {
            let (title, body) = this.update(cx, |page, cx| {
                let title = page.form.as_ref().map_or_else(
                    || "Операция".to_owned(),
                    |form| match (page.kind, form.id.is_some()) {
                        (TransactionKind::Income, true) => "Изменить доход".to_owned(),
                        (TransactionKind::Income, false) => "Новый доход".to_owned(),
                        (TransactionKind::Expense, true) => "Изменить расход".to_owned(),
                        (TransactionKind::Expense, false) => "Новый расход".to_owned(),
                    },
                );
                (title, page.render_form_dialog(cx))
            });
            dialog
                .title(title)
                .centered()
                .w(px(800.0))
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(body)
        });
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
                        Button::new("income-continue-editing")
                            .label("Вернуться к форме")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.confirm_discard = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("income-discard-changes")
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

    fn request_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_changes = self.form.as_ref().is_some_and(|form| {
            let current = FormValues {
                date: selected_date(form.date.read(cx)),
                amount: form.amount.read(cx).value().to_string(),
                comment: form.comment.read(cx).value().to_string(),
                currency: form.currency,
                category_id: form.category_id,
                expense_place_id: form.expense_place_id,
                card_id: form.card_id,
                affects_deposit_rate: form.affects_deposit_rate,
            };
            current.changed_from(&form.initial_values)
        });
        if has_changes {
            self.confirm_discard = true;
            cx.notify();
        } else {
            self.form = None;
            self.confirm_discard = false;
            window.close_dialog(cx);
            cx.notify();
        }
    }

    /// Активные категории плюс историческая категория редактируемой записи.
    fn rebuild_category_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let selectable: HashSet<i64> = self
            .categories
            .iter()
            .filter(|category| category.is_active || Some(category.id) == form.category_id)
            .map(|category| category.id)
            .collect();
        let entries: Vec<(i64, String)> = category_tree_labels(&self.categories)
            .into_iter()
            .filter(|(id, _)| selectable.contains(id))
            .collect();
        form.category_ids = entries.iter().map(|(id, _)| *id).collect();
        let selected = form
            .category_id
            .and_then(|id| form.category_ids.iter().position(|c| *c == id));
        let items: Vec<SharedString> = entries.into_iter().map(|(_, l)| l.into()).collect();
        form.category_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(
                selected.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            );
        });
    }

    fn rebuild_card_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let cards = eligible_cards(&self.cards, form.currency);
        form.card_ids = std::iter::once(None)
            .chain(cards.iter().map(|card| Some(card.id)))
            .collect();
        let items: Vec<SharedString> = std::iter::once("Без карты".to_owned())
            .chain(cards.iter().map(|card| card_label(card)))
            .map(SharedString::from)
            .collect();
        let selected = form
            .card_ids
            .iter()
            .position(|id| *id == form.card_id)
            .unwrap_or(0);
        form.card_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(Some(IndexPath::default().row(selected)), window, cx);
        });
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = self.form.as_ref() else {
            return false;
        };
        let parsed_date = selected_date(form.date.read(cx)).ok_or("Выберите дату");
        let amount_text = form.amount.read(cx).value().to_string();
        let comment = form.comment.read(cx).value().to_string();
        let date = match parsed_date {
            Ok(date) => date,
            Err(message) => {
                self.error(message, cx);
                return false;
            }
        };
        let amount = match parse_amount_expression(&amount_text) {
            Ok(amount) => amount,
            Err(error) => {
                self.error(error.to_string(), cx);
                return false;
            }
        };
        let Some(category_id) = form.category_id else {
            self.error("Выберите категорию", cx);
            return false;
        };
        let Some(currency) = form.currency else {
            self.error("Выберите валюту", cx);
            return false;
        };
        let result = finance_db::save_transaction_with_card_and_rate_effect_and_place(
            &self.database,
            self.kind,
            form.id,
            date,
            category_id,
            amount,
            currency,
            &comment,
            form.card_id,
            form.affects_deposit_rate,
            form.expense_place_id,
        );
        match result {
            Ok(_) => {
                self.form = None;
                self.refresh(cx);
                cx.notify();
                true
            }
            Err(error) => {
                self.error(error.to_string(), cx);
                false
            }
        }
    }

    fn confirm_delete(
        &mut self,
        transaction: &FinancialTransaction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = transaction.id;
        let kind = self.kind;
        let description = format!(
            "Удалить операцию от {} на сумму {} {}?",
            format_date(transaction.date),
            transaction.amount,
            transaction.currency.code()
        );
        let this = cx.entity();
        window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
            let this = this.clone();
            alert
                .title("Подтверждение удаления")
                .description(description.clone())
                .ok_text("Удалить")
                .cancel_text("Отмена")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |page, cx| {
                        match finance_db::delete_transaction(&page.database, kind, id) {
                            Ok(()) => page.refresh(cx),
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
        let this = cx.entity();
        let payment_label = payment_filter_label(self.kind);
        let filter = |label: &'static str, width: f32, control: AnyElement| {
            div()
                .w(px(width))
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_size(px(12.0)).child(label))
                .child(control)
        };

        // Полный текст выбранной категории для tooltip
        let category_tooltip = self
            .category_filter_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .and_then(|row| {
                let labels = self.category_filter_labels();
                labels.get(row).cloned()
            })
            .filter(|label| label != NONE_LABEL);

        let bank_tooltip = self
            .bank_filter_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .and_then(|row| self.bank_filter_labels().get(row).cloned())
            .filter(|label| label != NONE_LABEL);

        let card_tooltip = self
            .card_filter_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .and_then(|row| self.card_filter_labels().get(row).cloned())
            .filter(|label| label != NONE_LABEL && label != NO_CARD_LABEL);

        let place_tooltip = self
            .expense_place_filter_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .and_then(|row| row.checked_sub(1))
            .and_then(|row| self.expense_places.get(row))
            .map(expense_place_tooltip);

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
                        200.0,
                        Input::new(&self.search)
                            .cleanable(true)
                            .small()
                            .suffix(Icon::new(IconName::Search).small())
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Период",
                        140.0,
                        Select::new(&self.period_select)
                            .placeholder("Период")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "С",
                        150.0,
                        DatePicker::new(&self.from)
                            .placeholder("Выбрать дату")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "По",
                        150.0,
                        DatePicker::new(&self.to)
                            .placeholder("Выбрать дату")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Категория",
                        260.0,
                        div()
                            .id("category-filter-select")
                            .child(
                                Select::new(&self.category_filter_select)
                                    .placeholder("Категория")
                                    .small(),
                            )
                            .when_some(category_tooltip.clone(), |el, tooltip| {
                                el.tooltip(move |window, cx| {
                                    ComponentTooltip::new(tooltip.clone()).build(window, cx)
                                })
                            })
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Банк",
                        180.0,
                        div()
                            .id("bank-filter-select")
                            .child(
                                Select::new(&self.bank_filter_select)
                                    .placeholder("Банк")
                                    .small(),
                            )
                            .when_some(bank_tooltip.clone(), |el, tooltip| {
                                el.tooltip(move |window, cx| {
                                    ComponentTooltip::new(tooltip.clone()).build(window, cx)
                                })
                            })
                            .into_any_element(),
                    ))
                    .child(filter(
                        payment_label,
                        260.0,
                        div()
                            .id("card-filter-select")
                            .child(
                                Select::new(&self.card_filter_select)
                                    .placeholder(payment_label)
                                    .small(),
                            )
                            .when_some(card_tooltip.clone(), |el, tooltip| {
                                el.tooltip(move |window, cx| {
                                    ComponentTooltip::new(tooltip.clone()).build(window, cx)
                                })
                            })
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
                    .when(self.kind == TransactionKind::Expense, |filters| {
                        filters.child(filter(
                            EXPENSE_PLACE_LABEL,
                            220.0,
                            div()
                                .id("expense-place-filter-select")
                                .child(
                                    Select::new(&self.expense_place_filter_select)
                                        .placeholder(EXPENSE_PLACE_LABEL)
                                        .small(),
                                )
                                .when_some(place_tooltip.clone(), |el, tooltip| {
                                    el.tooltip(move |window, cx| {
                                        ComponentTooltip::new(tooltip.clone()).build(window, cx)
                                    })
                                })
                                .into_any_element(),
                        ))
                    }),
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
                            .on_click({
                                let this = this.clone();
                                move |_, _, cx| this.update(cx, |page, cx| page.refresh(cx))
                            }),
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
        let kind = self.kind;
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let amount_text = form.amount.read(cx).value().to_string();
        let rate_eligible = eligible_cards(&self.cards, form.currency)
            .iter()
            .any(|card| Some(card.id) == form.card_id && card.affects_deposit_rate);
        let legacy_debit_expense = form.id.is_some()
            && !form.balance_applied
            && self.cards.iter().any(|card| {
                Some(card.id) == form.card_id && card.card_type == PaymentCardType::Debit
            });
        let warning = cx.theme().warning;
        let total = (!amount_text.trim().is_empty())
            .then(|| parse_amount_expression(&amount_text).map_err(|error| error.to_string()));
        let card_missing_currency = form.card_id.is_some()
            && !eligible_cards(&self.cards, form.currency)
                .iter()
                .any(|card| Some(card.id) == form.card_id);
        let field = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_size(px(12.0)).child(label))
                .child(control)
        };

        let cell = |element: Div| element.flex_1().min_w_0();
        let mut card_field = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(px(12.0)).child(match kind {
                TransactionKind::Income => "Зачислить на карту",
                TransactionKind::Expense => "Оплачено картой",
            }))
            .child(Select::new(&form.card_select));
        if card_missing_currency {
            card_field = card_field.child(div().text_color(danger).child(match kind {
                TransactionKind::Income => {
                    "Выберите карту в валюте дохода или отмените выбор карты"
                }
                TransactionKind::Expense => {
                    "Выберите карту в валюте расхода или отмените выбор карты"
                }
            }));
        }
        let expense_place_field = field(
            EXPENSE_PLACE_LABEL,
            Select::new(&form.expense_place_select)
                .placeholder("Не указано")
                .into_any_element(),
        );

        let mut fields = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(cell(field(
                        "Дата",
                        DatePicker::new(&form.date)
                            .placeholder("Выбрать дату")
                            .into_any_element(),
                    )))
                    .child(cell(field(
                        "Сумма",
                        Input::new(&form.amount).into_any_element(),
                    )))
                    .child(cell(field(
                        "Валюта",
                        Select::new(&form.currency_select)
                            .placeholder("Выберите валюту")
                            .into_any_element(),
                    ))),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(cell(field(
                        "Категория",
                        Select::new(&form.category_select)
                            .placeholder("Выберите категорию")
                            .into_any_element(),
                    )))
                    .child(cell(card_field))
                    .when(kind == TransactionKind::Expense, |row| {
                        row.child(cell(expense_place_field))
                    }),
            );
        match kind {
            TransactionKind::Income => {
                fields = fields.child(div().text_xs().text_color(muted).child(
                    "Доступны дебетовые карты с указанной валютой. Для ранее созданных карт сначала укажите валюту в разделе «Платёжные карты».",
                ));
            }
            TransactionKind::Expense => {
                let enabled = rate_eligible || form.affects_deposit_rate;
                fields = fields
                    .child(
                        Checkbox::new("affects-rate")
                            .label("Учитывать расход при расчёте процентов по вкладам")
                            .checked(form.affects_deposit_rate)
                            .disabled(!enabled)
                            .on_click(cx.listener(|page, checked: &bool, _, cx| {
                                if let Some(form) = page.form.as_mut() {
                                    form.affects_deposit_rate = *checked;
                                }
                                cx.notify();
                            })),
                    )
                    .when(!rate_eligible, |f| {
                        f.child(div().text_xs().text_color(muted).child(
                            "Для учёта выберите карту с совпадающей валютой и включённым влиянием на ставку.",
                        ))
                    })
                    .child(div().text_xs().text_color(muted).child(
                        "Расход с дебетовой карты списывается с её баланса; кредитный лимит не меняется. Оценка процентов пока не рассчитывается.",
                    ))
                    .when(legacy_debit_expense, |f| {
                        f.child(div().text_xs().text_color(warning).child(
                            "Этот расход ранее не списывался с баланса. При сохранении будет списана его текущая сумма.",
                        ))
                    });
            }
        }
        fields = fields.child(field(
            "Комментарий",
            Textarea::new(&form.comment).into_any_element(),
        ));

        if !form.suggestions.is_empty() {
            let buttons = form.suggestions.iter().enumerate().map(|(index, text)| {
                let text = text.clone();
                Button::new(("suggestion", index))
                    .compact()
                    .label(text.clone())
                    .on_click(cx.listener(move |page, _, window, cx| {
                        if let Some(form) = page.form.as_mut() {
                            form.suggestions.clear();
                            let comment = form.comment.clone();
                            comment
                                .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                        }
                        cx.notify();
                    }))
            });
            fields = fields.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child("Ранее использованные комментарии:")
                    .children(buttons),
            );
        }
        if amount_text.contains('/') {
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(DIVISION_ROUNDING_WARNING),
            );
        }

        let mut panel = div().flex().flex_col().gap_3().p_4().child(fields);
        if let Some((message, true)) = &self.message {
            panel = panel.child(div().text_color(danger).child(message.clone()));
        }
        panel = panel
            .child(match total {
                Some(Ok(total)) => div().child(format!("Итого: {total}")),
                Some(Err(error)) => div().text_color(danger).child(error),
                None => div(),
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("income-form-save")
                            .primary()
                            .label("Сохранить")
                            .on_click(cx.listener(|page, _, window, cx| {
                                if page.save(cx) {
                                    page.confirm_discard = false;
                                    window.close_dialog(cx);
                                }
                            })),
                    )
                    .child(Button::new("income-form-cancel").label("Отмена").on_click(
                        cx.listener(|page, _, window, cx| page.request_cancel(window, cx)),
                    )),
            );
        panel.into_any_element()
    }

    fn render_table(&mut self) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(format!("Всего записей: {}", self.transactions.len()))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true).bordered(true)),
            )
    }

    fn render_footer(&self, cx: &App) -> impl IntoElement {
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let mut footer = div().flex().flex_col().gap_1();
        match (&self.balances, self.balance_period) {
            (Some(balances), Some((from, to))) => match self.primary_currency() {
                None => {
                    footer =
                        footer.child(div().text_color(danger).child(
                            "Сальдо не отображается: не удалось определить основную валюту.",
                        ));
                }
                Some(primary) => match balances.iter().find(|b| b.currency.code() == primary) {
                    None => {
                        footer = footer.child(div().text_color(danger).child(format!(
                                "Сальдо не отображается: основная валюта {primary} не поддерживается для операций (доступны RUB и USD)."
                            )));
                    }
                    Some(b) => {
                        let code = b.currency.code();
                        footer = footer
                                .child(div().font_weight(FontWeight::BOLD).child(format!(
                                    "{}: Доходы: {} {code} − Расходы: {} {code} = Сальдо: {} {code}",
                                    period_label(from, to),
                                    format_currency(&b.income, 2),
                                    format_currency(&b.expenses, 2),
                                    format_currency(&b.balance, 2),
                                )))
                                .child(div().text_color(muted).child(
                                    "Сальдо операций не включает начальные остатки и переводы и не подтверждает сумму денег на руках.",
                                ));
                    }
                },
            },
            _ => {
                footer = footer.child(
                    div()
                        .text_color(danger)
                        .child("Сальдо не рассчитано: обновите список после устранения ошибки."),
                );
            }
        }
        footer.child(self.list_total(danger))
    }

    fn list_total(&self, danger: Hsla) -> Div {
        let count = self.transactions.len();
        let Some(primary) = self.primary_currency() else {
            return div().child(format!(
                "Записей: {count}   Итого недоступно: основная валюта не определена"
            ));
        };
        let Some(currency) = Currency::ALL.into_iter().find(|c| c.code() == primary) else {
            return div().child(format!(
                "Записей: {count}   Итого недоступно: основная валюта {primary} не поддерживается для операций"
            ));
        };
        let total = self
            .transactions
            .iter()
            .filter(|t| t.currency == currency)
            .try_fold(Decimal::ZERO, |sum, t| sum.checked_add(t.amount));
        match total {
            Some(total) => div().child(format!("Записей: {count}   Итого: {} {primary}", format_currency(&total, 2))),
            None => div()
                .text_color(danger)
                .child("Не удалось рассчитать итог: сумма слишком велика"),
        }
    }
}

const INCOME_COLUMN_TITLES: [&str; 6] = [
    "Дата",
    "Категория",
    "Сумма",
    "Валюта",
    "Комментарии",
    "Действия",
];
const EXPENSE_COLUMN_TITLES: [&str; 7] = [
    "Дата",
    "Категория",
    "Сумма",
    "Валюта",
    EXPENSE_PLACE_LABEL,
    "Комментарии",
    "Действия",
];
const ACTIONS_COLUMN_WIDTH: f32 = 160.0;
const EXPENSE_PLACE_COLUMN_WIDTH: f32 = 180.0;
const DEFAULT_INCOME_COLUMN_WIDTHS: [f32; 6] =
    [115.0, 220.0, 135.0, 90.0, 320.0, ACTIONS_COLUMN_WIDTH];
const DEFAULT_EXPENSE_COLUMN_WIDTHS: [f32; 7] = [
    115.0,
    220.0,
    135.0,
    90.0,
    EXPENSE_PLACE_COLUMN_WIDTH,
    320.0,
    ACTIONS_COLUMN_WIDTH,
];
const MIN_COLUMN_WIDTH: f32 = 60.0;
const MAX_COLUMN_WIDTH: f32 = 1200.0;
const COL_DATE: usize = 0;
const COL_CATEGORY: usize = 1;
const COL_AMOUNT: usize = 2;
const COL_CURRENCY: usize = 3;
const COL_INCOME_COMMENT: usize = 4;
const COL_INCOME_ACTIONS: usize = 5;
const COL_EXPENSE_PLACE: usize = 4;
const COL_EXPENSE_COMMENT: usize = 5;
const COL_EXPENSE_ACTIONS: usize = 6;

fn column_titles(kind: TransactionKind) -> &'static [&'static str] {
    match kind {
        TransactionKind::Income => &INCOME_COLUMN_TITLES,
        TransactionKind::Expense => &EXPENSE_COLUMN_TITLES,
    }
}

fn actions_column(kind: TransactionKind) -> usize {
    match kind {
        TransactionKind::Income => COL_INCOME_ACTIONS,
        TransactionKind::Expense => COL_EXPENSE_ACTIONS,
    }
}

fn comment_column(kind: TransactionKind) -> usize {
    match kind {
        TransactionKind::Income => COL_INCOME_COMMENT,
        TransactionKind::Expense => COL_EXPENSE_COMMENT,
    }
}

fn default_column_widths(kind: TransactionKind) -> Vec<f32> {
    match kind {
        TransactionKind::Income => DEFAULT_INCOME_COLUMN_WIDTHS.to_vec(),
        TransactionKind::Expense => DEFAULT_EXPENSE_COLUMN_WIDTHS.to_vec(),
    }
}

fn column_widths_key(kind: TransactionKind) -> &'static str {
    match kind {
        TransactionKind::Income => "table_columns.income",
        TransactionKind::Expense => "table_columns.expense",
    }
}

fn format_column_widths(widths: &[f32]) -> String {
    widths
        .iter()
        .map(|width| width.round().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Разбирает сохранённые ширины; повреждённое значение отклоняется целиком.
fn parse_column_widths(value: &str, kind: TransactionKind) -> Option<Vec<f32>> {
    let mut widths = value
        .split(',')
        .map(|part| part.trim().parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()?;
    let valid_widths = widths
        .iter()
        .all(|width| width.is_finite() && (MIN_COLUMN_WIDTH..=MAX_COLUMN_WIDTH).contains(width));
    if !valid_widths {
        return None;
    }
    match kind {
        TransactionKind::Income if widths.len() == INCOME_COLUMN_TITLES.len() => Some(widths),
        TransactionKind::Expense if widths.len() == EXPENSE_COLUMN_TITLES.len() => Some(widths),
        TransactionKind::Expense if widths.len() == INCOME_COLUMN_TITLES.len() => {
            let comment_width = widths[COL_INCOME_COMMENT];
            let actions_width = widths[COL_INCOME_ACTIONS];
            widths.insert(COL_EXPENSE_PLACE, EXPENSE_PLACE_COLUMN_WIDTH);
            widths[COL_EXPENSE_COMMENT] = comment_width;
            widths[COL_EXPENSE_ACTIONS] = actions_width;
            Some(widths)
        }
        _ => None,
    }
}

/// Ширины из БД или значения по умолчанию; ошибка чтения или формата логируется и не прерывает запуск.
fn load_column_widths(database: &Database, kind: TransactionKind) -> Vec<f32> {
    match finance_db::get_app_setting(database, column_widths_key(kind)) {
        Ok(Some(value)) => parse_column_widths(&value, kind)
            .map(|widths| column_widths_with_fixed_actions(widths, kind))
            .unwrap_or_else(|| {
                eprintln!(
                    "Некорректная сохранённая ширина колонок; использованы значения по умолчанию"
                );
                default_column_widths(kind)
            }),
        Ok(None) => default_column_widths(kind),
        Err(error) => {
            eprintln!("Не удалось прочитать ширину колонок: {error}");
            default_column_widths(kind)
        }
    }
}

fn column_widths_with_fixed_actions(mut widths: Vec<f32>, kind: TransactionKind) -> Vec<f32> {
    if let Some(width) = widths.get_mut(actions_column(kind)) {
        *width = ACTIONS_COLUMN_WIDTH;
    }
    widths
}

fn default_order(a: &FinancialTransaction, b: &FinancialTransaction) -> std::cmp::Ordering {
    b.date.cmp(&a.date).then(b.id.cmp(&a.id))
}

/// Сортирует полный набор строк; суммы сравниваются как `Decimal`, а не как текст.
fn sort_transactions(rows: &mut [FinancialTransaction], sort: Option<(usize, ColumnSort)>) {
    let Some((column, direction)) = sort else {
        rows.sort_by(default_order);
        return;
    };
    rows.sort_by(|a, b| {
        let ordering = match column {
            COL_DATE => a.date.cmp(&b.date),
            COL_CATEGORY => a
                .category_name
                .to_lowercase()
                .cmp(&b.category_name.to_lowercase()),
            COL_AMOUNT => a
                .amount
                .cmp(&b.amount)
                .then_with(|| a.currency.code().cmp(b.currency.code())),
            _ => std::cmp::Ordering::Equal,
        };
        let ordering = if direction == ColumnSort::Descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| default_order(a, b))
    });
}

/// Делегат `DataTable` для доходов и расходов: хранит строки, ширины и сортировку.
pub(crate) struct TransactionsTable {
    page: WeakEntity<TransactionsPage>,
    kind: TransactionKind,
    rows: Vec<FinancialTransaction>,
    widths: Vec<f32>,
    sort: Option<(usize, ColumnSort)>,
}

impl TransactionsTable {
    fn new(page: WeakEntity<TransactionsPage>, kind: TransactionKind, widths: Vec<f32>) -> Self {
        Self {
            page,
            kind,
            rows: Vec::new(),
            widths: column_widths_with_fixed_actions(widths, kind),
            sort: None,
        }
    }

    fn set_rows(&mut self, rows: Vec<FinancialTransaction>) {
        self.rows = rows;
        sort_transactions(&mut self.rows, self.sort);
    }

    fn set_widths(&mut self, widths: &[f32]) {
        if widths.len() == self.widths.len() {
            self.widths = column_widths_with_fixed_actions(widths.to_vec(), self.kind);
        }
    }
}

impl TableDelegate for TransactionsTable {
    fn columns_count(&self, _: &App) -> usize {
        column_titles(self.kind).len()
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
        let titles = column_titles(self.kind);
        let actions_column = actions_column(self.kind);
        div()
            .w_full()
            .h_full()
            .when(col_ix > COL_DATE && col_ix < actions_column, |cell| {
                cell.border_l_1().border_color(border)
            })
            .child(titles[col_ix])
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let is_actions = col_ix == actions_column(self.kind);
        let titles = column_titles(self.kind);
        let column = Column::new(titles[col_ix], titles[col_ix])
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
        let column = match col_ix {
            COL_DATE | COL_CATEGORY | COL_AMOUNT => {
                let direction = match self.sort {
                    Some((sorted, direction)) if sorted == col_ix => direction,
                    _ => ColumnSort::Default,
                };
                column.sort(direction)
            }
            _ => column,
        };
        if col_ix == COL_AMOUNT {
            column.text_right()
        } else {
            column
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
        sort_transactions(&mut self.rows, self.sort);
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
            .child("Операций по выбранным условиям нет.")
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let transaction = &self.rows[row_ix];
        let id = transaction.id;
        let content = match col_ix {
            COL_DATE => div()
                .child(format_date(transaction.date))
                .into_any_element(),
            COL_CATEGORY => div()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(transaction.category_name.clone())
                .into_any_element(),
            COL_AMOUNT => div()
                .w_full()
                .text_right()
                .child(format_currency(&transaction.amount, 2))
                .into_any_element(),
            COL_CURRENCY => div()
                .child(transaction.currency.code().to_owned())
                .into_any_element(),
            COL_EXPENSE_PLACE if self.kind == TransactionKind::Expense => {
                let place_name = transaction
                    .expense_place_name
                    .clone()
                    .unwrap_or_else(|| "—".to_owned());
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(place_name)
                    .into_any_element()
            }
            column if column == comment_column(self.kind) => {
                let comment = transaction.comment.clone();
                if comment.is_empty() {
                    div().into_any_element()
                } else {
                    div()
                        .id(("transaction-comment", id as usize))
                        .w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .child(comment_preview(&comment))
                        .on_click(move |_, window, cx| {
                            let comment = comment.clone();
                            window.open_dialog(cx, move |dialog: Dialog, _, _| {
                                dialog.title("Комментарий").centered().child(
                                    div()
                                        .id("transaction-comment-dialog-body")
                                        .max_h(px(420.0))
                                        .overflow_y_scroll()
                                        .overflow_x_scroll()
                                        .child(comment.clone()),
                                )
                            });
                        })
                        .into_any_element()
                }
            }
            column if column == actions_column(self.kind) => {
                let edit_page = self.page.clone();
                let delete_page = self.page.clone();
                let edit_target = transaction.clone();
                let delete_target = transaction.clone();
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
                                let _ = edit_page.update(cx, |page, cx| {
                                    page.open_form(Some(&edit_target), window, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(("delete", id as usize))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .on_click(move |_, window, cx| {
                                let _ = delete_page.update(cx, |page, cx| {
                                    page.confirm_delete(&delete_target, window, cx)
                                });
                            }),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        };
        let border = cx.theme().border;
        let actions_column = actions_column(self.kind);
        div()
            .w_full()
            .h_full()
            .when(col_ix > COL_DATE && col_ix < actions_column, |cell| {
                cell.border_l_1().border_color(border)
            })
            .child(content)
            .into_any_element()
    }
}

pub(crate) fn comment_preview(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default();
    let mut chars = first_line.chars();
    let mut preview = chars.by_ref().take(25).collect::<String>();

    if chars.next().is_some() || text.contains('\n') || text.contains('\r') {
        preview.push('…');
    }

    preview
}

pub(crate) fn period_label(from: Option<NaiveDate>, to: Option<NaiveDate>) -> String {
    match (from, to) {
        (None, None) => "За весь период".to_owned(),
        (Some(from), Some(to)) => {
            format!("За период с {} по {}", format_date(from), format_date(to))
        }
        (Some(from), None) => format!("За период с {}", format_date(from)),
        (None, Some(to)) => format!("За период по {}", format_date(to)),
    }
}

impl Render for TransactionsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_active_category = self.categories.iter().any(|category| category.is_active);
        let danger = cx.theme().danger;
        let (title, add_label) = match self.kind {
            TransactionKind::Income => ("Доходы", "Добавить доход"),
            TransactionKind::Expense => ("Расходы", "Добавить расход"),
        };
        let mut root = div().size_full().flex().flex_col().gap_2().p_3();
        let header = div()
            .flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(24.0))
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("new-transaction")
                            .primary()
                            .large()
                            .label(add_label)
                            .icon(IconName::Plus)
                            .disabled(self.form.is_some())
                            .on_click(
                                cx.listener(|page, _, window, cx| page.open_form(None, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("refresh-transactions")
                            .secondary()
                            .large()
                            .label("Обновить")
                            .icon(IconName::Replace)
                            .on_click(cx.listener(|page, _, window, cx| page.reload(window, cx))),
                    ),
            );
        root = root.child(header);
        root = root.child(self.render_filters(cx));
        if self.form.is_none() {
            if let Some((message, true)) = &self.message {
                root = root.child(div().text_color(danger).child(message.clone()));
            }
        }
        if !has_active_category {
            root = root.child("Сначала добавьте категорию в разделе «Справочники».");
        }
        root.child(self.render_table())
            .child(self.render_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        card_filter_from_index, card_filter_index, cards_for_bank,
        column_widths_with_fixed_actions, comment_preview, format_column_widths,
        parse_column_widths, payment_filter_label, payment_filter_labels, sort_transactions,
        update_bank_filter_selection, BankFilterItem, FormValues, PlaceFilterItem,
        ACTIONS_COLUMN_WIDTH, COL_INCOME_ACTIONS, EXPENSE_COLUMN_TITLES, EXPENSE_PLACE_LABEL,
        INCOME_PAYMENT_FILTER_LABEL, NONE_LABEL, NO_CARD_LABEL, PAYMENT_FILTER_LABEL,
    };
    use crate::gpui::support::card_label;
    use chrono::NaiveDate;
    use finance_core::FinancialTransaction;
    use finance_core::{
        Currency, PaymentCardType, PaymentDocument, PaymentDocumentFilter, PaymentSystem,
        TransactionKind,
    };
    use gpui_kit::component::select::SelectItem as _;
    use gpui_kit::component::table::ColumnSort;
    use rust_decimal::Decimal;

    fn payment_document(
        id: i64,
        name: Option<&str>,
        card_type: PaymentCardType,
        currency: Option<Currency>,
    ) -> PaymentDocument {
        PaymentDocument {
            id,
            name: name.map(str::to_owned),
            bank_id: 1,
            bank_name: "Альфа-Банк".to_owned(),
            payment_system: PaymentSystem::Mir,
            card_type,
            amount: Decimal::ONE,
            currency,
            due_date: None,
            grace_period_days: None,
            next_payment_amount: None,
            affects_deposit_rate: false,
            min_monthly_spend: None,
            linked_savings: Vec::new(),
        }
    }

    #[test]
    fn card_label_includes_name_and_preserves_card_details() {
        let debit = payment_document(
            7,
            Some("Повседневная"),
            PaymentCardType::Debit,
            Some(Currency::Rub),
        );
        let credit = payment_document(
            8,
            Some("Повседневная"),
            PaymentCardType::Credit,
            Some(Currency::Usd),
        );

        assert_eq!(
            card_label(&debit),
            "Повседневная — Альфа-Банк — MIR #7 (дебетовая, RUB)"
        );
        assert_eq!(
            card_label(&credit),
            "Повседневная — Альфа-Банк — MIR #8 (кредитная, USD)"
        );
        assert_ne!(card_label(&debit), card_label(&credit));
    }

    #[test]
    fn card_label_keeps_existing_format_for_unnamed_cards() {
        let card = payment_document(9, None, PaymentCardType::Debit, None);

        assert_eq!(
            card_label(&card),
            "Альфа-Банк — MIR #9 (дебетовая, валюта не указана)"
        );
    }

    #[test]
    fn card_filter_options_include_all_cards_without_a_bank_filter() {
        let first = payment_document(10, None, PaymentCardType::Debit, Some(Currency::Rub));
        let mut second = payment_document(11, None, PaymentCardType::Credit, None);
        second.bank_id = 2;

        let cards = [first, second];
        let available = cards_for_bank(&cards, None);

        assert_eq!(
            available.iter().map(|card| card.id).collect::<Vec<_>>(),
            [10, 11]
        );
    }

    #[test]
    fn card_filter_options_only_include_cards_from_the_selected_bank() {
        let first = payment_document(10, None, PaymentCardType::Debit, Some(Currency::Rub));
        let mut second = payment_document(11, None, PaymentCardType::Credit, None);
        second.bank_id = 2;

        let cards = [first, second];
        let available = cards_for_bank(&cards, Some(2));

        assert_eq!(
            available.iter().map(|card| card.id).collect::<Vec<_>>(),
            [11]
        );
    }

    #[test]
    fn changing_bank_filter_clears_the_selected_card() {
        let mut bank_filter = Some(1);
        let mut card_filter = PaymentDocumentFilter::Card(10);

        let changed = update_bank_filter_selection(&mut bank_filter, &mut card_filter, Some(2));

        assert!(changed);
        assert_eq!(bank_filter, Some(2));
        assert_eq!(card_filter, PaymentDocumentFilter::All);
    }

    #[test]
    fn payment_filter_labels_place_no_card_after_all() {
        let card = payment_document(10, None, PaymentCardType::Debit, Some(Currency::Rub));

        let labels = payment_filter_labels(&[card], None);

        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0], NONE_LABEL);
        assert_eq!(labels[1], NO_CARD_LABEL);
        assert_eq!(PAYMENT_FILTER_LABEL, "Способ оплаты");
        assert!(labels[2].contains("MIR #10"));
    }

    #[test]
    fn payment_filter_label_uses_receiving_wording_for_income() {
        assert_eq!(
            payment_filter_label(TransactionKind::Income),
            INCOME_PAYMENT_FILTER_LABEL
        );
        assert_eq!(INCOME_PAYMENT_FILTER_LABEL, "Способ получения");
    }

    #[test]
    fn payment_filter_label_uses_payment_wording_for_expense() {
        assert_eq!(
            payment_filter_label(TransactionKind::Expense),
            PAYMENT_FILTER_LABEL
        );
        assert_eq!(PAYMENT_FILTER_LABEL, "Способ оплаты");
    }

    #[test]
    fn bank_filter_item_keeps_full_label_for_tooltip() {
        let item = BankFilterItem {
            id: 3,
            label: "Альфа-Банк".to_owned(),
        };

        assert_eq!(item.label, "Альфа-Банк");
        assert_eq!(item.id, 3);
    }

    #[test]
    fn expense_place_filter_item_keeps_full_label_for_tooltip() {
        let item = PlaceFilterItem {
            id: 2,
            label: "Супермаркет у дома".to_owned(),
            tooltip: Some("Супермаркет у дома · #5\nКомментарий: удобный подъезд".to_owned()),
        };

        assert_eq!(item.title().to_string(), "Супермаркет у дома");
        assert_eq!(item.value(), "Супермаркет у дома");
        assert_eq!(item.id, 2);
        assert_eq!(
            item.tooltip.as_deref(),
            Some("Супермаркет у дома · #5\nКомментарий: удобный подъезд")
        );
        assert_eq!(EXPENSE_PLACE_LABEL, "Место покупки");
        assert_eq!(EXPENSE_COLUMN_TITLES[4], EXPENSE_PLACE_LABEL);
    }

    #[test]
    fn card_filter_indexes_map_all_three_options() {
        let card_ids = [10, 11];

        assert_eq!(
            card_filter_from_index(0, &card_ids),
            PaymentDocumentFilter::All
        );
        assert_eq!(
            card_filter_from_index(1, &card_ids),
            PaymentDocumentFilter::WithoutCard
        );
        assert_eq!(
            card_filter_from_index(2, &card_ids),
            PaymentDocumentFilter::Card(10)
        );
        assert_eq!(
            card_filter_from_index(3, &card_ids),
            PaymentDocumentFilter::Card(11)
        );
        assert_eq!(
            card_filter_from_index(9, &card_ids),
            PaymentDocumentFilter::All
        );

        for filter in [
            PaymentDocumentFilter::All,
            PaymentDocumentFilter::WithoutCard,
            PaymentDocumentFilter::Card(10),
        ] {
            let index = card_filter_index(filter, &card_ids).expect("filter has an option");
            assert_eq!(card_filter_from_index(index, &card_ids), filter);
        }
        assert_eq!(
            card_filter_index(PaymentDocumentFilter::Card(99), &card_ids),
            None
        );
    }

    #[test]
    fn form_values_are_unchanged_when_they_match_the_initial_state() {
        let initial = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 20).expect("valid test date")),
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            expense_place_id: None,
            card_id: None,
            affects_deposit_rate: false,
        };

        assert!(!initial.changed_from(&initial));
    }

    #[test]
    fn form_values_detect_a_changed_amount() {
        let initial = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 20).expect("valid test date")),
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            expense_place_id: None,
            card_id: None,
            affects_deposit_rate: false,
        };
        let edited = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 20).expect("valid test date")),
            amount: "200".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            expense_place_id: None,
            card_id: None,
            affects_deposit_rate: false,
        };

        assert!(edited.changed_from(&initial));
    }

    #[test]
    fn form_values_detect_a_changed_expense_place() {
        let initial = FormValues {
            date: None,
            amount: "100".to_owned(),
            comment: String::new(),
            currency: None,
            category_id: Some(1),
            expense_place_id: Some(2),
            card_id: None,
            affects_deposit_rate: false,
        };
        let edited = FormValues {
            date: initial.date,
            amount: initial.amount.clone(),
            comment: initial.comment.clone(),
            currency: initial.currency,
            category_id: initial.category_id,
            expense_place_id: None,
            card_id: initial.card_id,
            affects_deposit_rate: initial.affects_deposit_rate,
        };

        assert!(edited.changed_from(&initial));
    }

    #[test]
    fn form_values_detect_a_changed_date() {
        let initial = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 20).expect("valid test date")),
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            expense_place_id: None,
            card_id: None,
            affects_deposit_rate: false,
        };
        let edited = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 21).expect("valid test date")),
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            expense_place_id: None,
            card_id: None,
            affects_deposit_rate: false,
        };

        assert!(edited.changed_from(&initial));
    }

    #[test]
    fn form_values_detect_a_cleared_date() {
        let initial = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 20).expect("valid test date")),
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            expense_place_id: None,
            card_id: None,
            affects_deposit_rate: false,
        };
        let edited = FormValues {
            date: None,
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            expense_place_id: None,
            card_id: None,
            affects_deposit_rate: false,
        };

        assert!(edited.changed_from(&initial));
    }

    #[test]
    fn comment_preview_keeps_short_single_line_text_unchanged() {
        assert_eq!(
            comment_preview("Короткий комментарий"),
            "Короткий комментарий"
        );
    }

    #[test]
    fn comment_preview_truncates_after_25_unicode_characters() {
        assert_eq!(
            comment_preview("Комментарий очень длинный, дальше текст"),
            "Комментарий очень длинный…"
        );
    }

    #[test]
    fn comment_preview_adds_ellipsis_for_multiline_text() {
        assert_eq!(
            comment_preview("Первая строка\nВторая строка"),
            "Первая строка…"
        );
        assert_eq!(
            comment_preview("Первая строка\r\nВторая строка"),
            "Первая строка…"
        );
    }

    fn transaction(id: i64, day: u32, category: &str, amount: &str) -> FinancialTransaction {
        FinancialTransaction {
            id,
            date: NaiveDate::from_ymd_opt(2024, 2, day).unwrap(),
            category_id: 1,
            category_name: category.to_owned(),
            amount: amount.parse().unwrap(),
            currency: Currency::Rub,
            comment: String::new(),
            expense_place_id: None,
            expense_place_name: None,
            payment_document_id: None,
            affects_deposit_rate: false,
            payment_document_balance_applied: false,
        }
    }

    fn ids(rows: &[FinancialTransaction]) -> Vec<i64> {
        rows.iter().map(|row| row.id).collect()
    }

    #[test]
    fn amount_sort_is_numeric_not_lexicographic() {
        let mut rows = vec![
            transaction(1, 1, "А", "9.5"),
            transaction(2, 2, "Б", "100"),
            transaction(3, 3, "В", "20"),
        ];
        sort_transactions(&mut rows, Some((2, ColumnSort::Ascending)));
        assert_eq!(ids(&rows), [1, 3, 2]);
        sort_transactions(&mut rows, Some((2, ColumnSort::Descending)));
        assert_eq!(ids(&rows), [2, 3, 1]);
    }

    #[test]
    fn date_and_category_sorts_and_default_order() {
        let mut rows = vec![
            transaction(1, 5, "б", "1"),
            transaction(2, 1, "А", "1"),
            transaction(3, 9, "В", "1"),
        ];
        sort_transactions(&mut rows, Some((0, ColumnSort::Ascending)));
        assert_eq!(ids(&rows), [2, 1, 3]);
        sort_transactions(&mut rows, Some((1, ColumnSort::Ascending)));
        assert_eq!(ids(&rows), [2, 1, 3]);
        sort_transactions(&mut rows, None);
        assert_eq!(ids(&rows), [3, 1, 2]);
    }

    #[test]
    fn column_widths_round_trip_and_reject_corrupt_values() {
        let widths = [100.0, 200.0, 130.0, 90.0, 300.0, 180.0];
        let text = format_column_widths(&widths);
        assert_eq!(
            parse_column_widths(&text, TransactionKind::Income).as_deref(),
            Some(&widths[..])
        );
        assert_eq!(parse_column_widths("1,2,3", TransactionKind::Income), None);
        assert_eq!(
            parse_column_widths("100,200,x,90,300,180", TransactionKind::Income),
            None
        );
        assert_eq!(
            parse_column_widths("100,200,130,90,300,5000", TransactionKind::Income),
            None
        );
        assert_eq!(
            parse_column_widths("100,200,130,90,300,NaN", TransactionKind::Income),
            None
        );
    }

    #[test]
    fn expense_column_widths_migrate_legacy_comment_width() {
        let legacy_widths = [100.0, 200.0, 130.0, 90.0, 300.0, 180.0];
        let migrated = parse_column_widths(
            &format_column_widths(&legacy_widths),
            TransactionKind::Expense,
        )
        .expect("legacy expense column widths should migrate");

        assert_eq!(migrated, [100.0, 200.0, 130.0, 90.0, 180.0, 300.0, 180.0]);
    }

    #[test]
    fn action_column_uses_fixed_width_in_saved_settings() {
        let saved_widths = vec![100.0, 200.0, 130.0, 90.0, 300.0, 190.0];
        let persisted = format_column_widths(&saved_widths);
        let parsed =
            parse_column_widths(&persisted, TransactionKind::Income).expect("valid saved widths");
        let widths = column_widths_with_fixed_actions(parsed, TransactionKind::Income);

        assert_eq!(
            &widths[..COL_INCOME_ACTIONS],
            &saved_widths[..COL_INCOME_ACTIONS]
        );
        assert_eq!(widths[COL_INCOME_ACTIONS], ACTIONS_COLUMN_WIDTH);
    }
}
