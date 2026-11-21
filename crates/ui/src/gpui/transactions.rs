//! Страницы «Доходы» и «Расходы» на `gpui-kit`. Поведение соответствует
//! `transactions_page.rs`; SQL и финансовые правила остаются в `finance-db` и
//! `finance-core`.

use std::collections::HashSet;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use finance_core::{
    parse_amount_expression, summarize_transaction_balance, Bank, Category, Currency,
    CurrencyDefinition, FinancialTransaction, PaymentCardType, PaymentDocument, TransactionBalance,
    TransactionKind,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_decimal::Decimal;

use super::support::category_tree_labels;
use super::support::{comment_suggestions, DIVISION_ROUNDING_WARNING};
use super::support::{date_period_range, DatePeriod, DatePeriodRange};

const DATE_FORMAT: &str = "%Y-%m-%d";
const ROW_HEIGHT: f32 = 40.0;
const NONE_LABEL: &str = "Все";

type Choice = Entity<SelectState<Vec<SharedString>>>;

fn choice(
    items: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<TransactionsPage>,
) -> Choice {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
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

fn card_label(card: &PaymentDocument) -> String {
    let card_type = match card.card_type {
        PaymentCardType::Debit => "дебетовая",
        PaymentCardType::Credit => "кредитная",
    };
    let details = format!(
        "{} — {} #{} ({card_type}, {})",
        card.bank_name,
        card.payment_system.code(),
        card.id,
        card.currency.map_or("валюта не указана", Currency::code)
    );
    match card
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(name) => format!("{name} — {details}"),
        None => details,
    }
}

/// Карты, доступные для выбора при данной валюте операции.
fn eligible_cards(cards: &[PaymentDocument], currency: Option<Currency>) -> Vec<&PaymentDocument> {
    cards
        .iter()
        .filter(|card| card.currency.is_some() && card.currency == currency)
        .collect()
}

fn cards_for_bank(cards: &[PaymentDocument], bank_id: Option<i64>) -> Vec<&PaymentDocument> {
    cards
        .iter()
        .filter(|card| bank_id.is_none_or(|bank_id| card.bank_id == bank_id))
        .collect()
}

fn update_bank_filter_selection(
    bank_filter: &mut Option<i64>,
    card_filter: &mut Option<i64>,
    next_bank: Option<i64>,
) -> bool {
    if *bank_filter == next_bank {
        return false;
    }
    *bank_filter = next_bank;
    *card_filter = None;
    true
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
    banks: Vec<Bank>,
    currencies: Vec<CurrencyDefinition>,
    cards: Vec<PaymentDocument>,
    filter_cards: Vec<PaymentDocument>,
    comment_history: Vec<String>,
    transactions: Vec<FinancialTransaction>,
    balances: Option<[TransactionBalance; 2]>,
    balance_period: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
    message: Option<(String, bool)>,
    search: Entity<InputState>,
    from: Entity<DatePickerState>,
    to: Entity<DatePickerState>,
    category_filter: Option<i64>,
    category_filter_ids: Vec<i64>,
    category_filter_select: Choice,
    bank_filter: Option<i64>,
    bank_filter_ids: Vec<i64>,
    bank_filter_select: Choice,
    card_filter: Option<i64>,
    card_filter_ids: Vec<i64>,
    card_filter_select: Choice,
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
        let category_filter_select = choice(vec![NONE_LABEL.to_owned()], 0, window, cx);
        let bank_filter_select = choice(vec![NONE_LABEL.to_owned()], 0, window, cx);
        let card_filter_select = choice(vec![NONE_LABEL.to_owned()], 0, window, cx);
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
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.category_filter = value
                    .as_ref()
                    .and_then(|label| {
                        this.category_filter_labels()
                            .iter()
                            .position(|l| l == label.as_ref())
                    })
                    .and_then(|position| position.checked_sub(1))
                    .and_then(|position| this.category_filter_ids.get(position).copied());
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &bank_filter_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                let SelectEvent::Confirm(value) = event;
                let next_bank = value
                    .as_ref()
                    .and_then(|label| {
                        this.bank_filter_labels()
                            .iter()
                            .position(|item| item == label.as_ref())
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
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.card_filter = value
                    .as_ref()
                    .and_then(|label| {
                        this.card_filter_labels()
                            .iter()
                            .position(|item| item == label.as_ref())
                    })
                    .and_then(|position| position.checked_sub(1))
                    .and_then(|position| this.card_filter_ids.get(position).copied());
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

        let mut page = Self {
            database,
            kind,
            categories: Vec::new(),
            banks: Vec::new(),
            currencies: Vec::new(),
            cards: Vec::new(),
            filter_cards: Vec::new(),
            comment_history: Vec::new(),
            transactions: Vec::new(),
            balances: None,
            balance_period: None,
            message: None,
            search,
            from,
            to,
            category_filter: None,
            category_filter_ids: Vec::new(),
            category_filter_select,
            bank_filter: None,
            bank_filter_ids: Vec::new(),
            bank_filter_select,
            card_filter: None,
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

    fn bank_filter_labels(&self) -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(self.banks.iter().map(|bank| bank.name.clone()))
            .collect()
    }

    fn card_filter_labels(&self) -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(
                cards_for_bank(&self.filter_cards, self.bank_filter)
                    .into_iter()
                    .map(card_label),
            )
            .collect()
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
        let items: Vec<SharedString> = labels.into_iter().map(SharedString::from).collect();
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
        self.banks = banks;
        self.filter_cards = cards;
        if self
            .bank_filter
            .is_some_and(|bank_id| !self.banks.iter().any(|bank| bank.id == bank_id))
        {
            self.bank_filter = None;
            self.card_filter = None;
        }
        if self.card_filter.is_some_and(|card_id| {
            !cards_for_bank(&self.filter_cards, self.bank_filter)
                .iter()
                .any(|card| card.id == card_id)
        }) {
            self.card_filter = None;
        }

        self.bank_filter_ids = self.banks.iter().map(|bank| bank.id).collect();
        let bank_items: Vec<SharedString> = self
            .bank_filter_labels()
            .into_iter()
            .map(SharedString::from)
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
        let selected_card_index = match self.card_filter {
            Some(card_id) => self
                .card_filter_ids
                .iter()
                .position(|candidate| *candidate == card_id)
                .map_or_else(
                    || {
                        self.card_filter = None;
                        0
                    },
                    |position| position + 1,
                ),
            None => 0,
        };
        let items: Vec<SharedString> = self
            .card_filter_labels()
            .into_iter()
            .map(SharedString::from)
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
        match finance_db::list_transactions_filtered(
            &self.database,
            self.kind,
            from,
            to,
            self.category_filter,
            self.currency_filter,
            &search,
            self.bank_filter,
            self.card_filter,
        ) {
            Ok(transactions) => self.transactions = transactions,
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
            || self.bank_filter.is_some()
            || self.card_filter.is_some()
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
        self.bank_filter = None;
        self.card_filter = None;
        self.currency_filter = None;
        self.sync_card_filter_options(window, cx);
        for select in [
            &self.category_filter_select,
            &self.bank_filter_select,
            &self.card_filter_select,
            &self.currency_filter_select,
            &self.period_select,
        ] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
            });
        }
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
        if !self.load_form_sources(cx) {
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
        let date =
            existing.map_or_else(|| Local::now().date_naive(), |transaction| transaction.date);
        let initial_values = FormValues {
            date: Some(date),
            amount: existing.map_or_else(String::new, |transaction| transaction.amount.to_string()),
            comment: existing.map_or_else(String::new, |transaction| transaction.comment.clone()),
            currency,
            category_id,
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
                        let suggestions = comment_suggestions(&this.comment_history, &text);
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
        let result = finance_db::save_transaction_with_card_and_rate_effect(
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
            transaction.date.format("%d.%m.%Y"),
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
                        Select::new(&self.category_filter_select)
                            .placeholder("Категория")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Банк",
                        180.0,
                        Select::new(&self.bank_filter_select)
                            .placeholder("Банк")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Платёжная карта",
                        260.0,
                        Select::new(&self.card_filter_select)
                            .placeholder("Платёжная карта")
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
                    .child(cell(card_field)),
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

    fn render_table(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let stripe = cx.theme().muted;
        let count = self.transactions.len();
        let headers = [
            "Дата",
            "Категория",
            "Сумма",
            "Валюта",
            "Комментарии",
            "Действия",
        ];
        let header = div()
            .flex()
            .w_full()
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_1()
            .border_color(border)
            .font_weight(FontWeight::BOLD)
            .children(
                headers
                    .into_iter()
                    .enumerate()
                    .map(|(index, title)| table_cell(index, title.into(), border)),
            );
        let body = if count == 0 {
            div()
                .w_full()
                .p_4()
                .child("Операций по выбранным условиям нет.")
                .into_any_element()
        } else {
            uniform_list(
                "transactions-rows",
                count,
                cx.processor(move |page, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| page.render_row(index, stripe, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .w_full()
            .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .w_full()
            .child(header)
            .child(body)
    }

    fn render_row(
        &mut self,
        index: usize,
        stripe: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let transaction = self.transactions[index].clone();
        let border = cx.theme().border;
        let id = transaction.id;
        let values = vec![
            transaction.date.format("%d.%m.%Y").to_string(),
            transaction.category_name.clone(),
            transaction.amount.to_string(),
            transaction.currency.code().to_owned(),
            transaction.comment.clone(),
        ];
        let edit_target = transaction.clone();
        let delete_target = transaction;
        div()
            .id(("transaction-row", id as usize))
            .flex()
            .w_full()
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_l_1()
            .border_r_1()
            .border_b_1()
            .border_color(border)
            .when(index % 2 == 1, |row| row.bg(stripe))
            .children(values.into_iter().enumerate().map(|(column, value)| {
                if column == 4 && !value.is_empty() {
                    comment_cell(id, value, cx).into_any_element()
                } else {
                    table_cell(column, value, border).into_any_element()
                }
            }))
            .child(
                div()
                    .w(px(ACTIONS_COLUMN_WIDTH))
                    .h_full()
                    .flex()
                    .items_center()
                    .border_l_1()
                    .border_color(border)
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new(("edit", id as usize))
                            .xsmall()
                            .label("Изменить")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.open_form(Some(&edit_target), window, cx)
                            })),
                    )
                    .child(
                        Button::new(("delete", id as usize))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.confirm_delete(&delete_target, window, cx)
                            })),
                    ),
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
                                    b.income,
                                    b.expenses,
                                    b.balance,
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
            Some(total) => div().child(format!("Записей: {count}   Итого: {total} {primary}")),
            None => div()
                .text_color(danger)
                .child("Не удалось рассчитать итог: сумма слишком велика"),
        }
    }
}

const FIXED_COLUMN_WIDTHS: [f32; 4] = [115.0, 220.0, 135.0, 90.0];
const ACTIONS_COLUMN_WIDTH: f32 = 180.0;

fn cell(width: f32, text: String, border: Hsla, separator: bool) -> Div {
    div()
        .w(px(width))
        .h_full()
        .flex()
        .items_center()
        .when(separator, |cell| cell.border_l_1().border_color(border))
        .px_2()
        .overflow_hidden()
        .child(text)
}

fn table_cell(index: usize, text: String, border: Hsla) -> Div {
    match index {
        0..=3 => cell(FIXED_COLUMN_WIDTHS[index], text, border, index > 0),
        4 => div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .border_l_1()
            .border_color(border)
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(text),
        _ => div()
            .w(px(ACTIONS_COLUMN_WIDTH))
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .border_l_1()
            .border_color(border)
            .justify_end()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(text),
    }
}

fn comment_cell(id: i64, text: String, cx: &mut Context<TransactionsPage>) -> AnyElement {
    let preview = comment_preview(&text);
    div()
        .id(("income-comment", id as usize))
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .items_center()
        .border_l_1()
        .border_color(cx.theme().border)
        .px_2()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(preview)
        .cursor_pointer()
        .on_click(cx.listener(move |_, _, window, cx| {
            let comment = text.clone();
            window.open_dialog(cx, move |dialog: Dialog, _, _| {
                dialog.title("Комментарий").centered().child(
                    div()
                        .id("income-comment-dialog-body")
                        .max_h(px(420.0))
                        .overflow_y_scroll()
                        .overflow_x_scroll()
                        .child(comment.clone()),
                )
            });
        }))
        .into_any_element()
}

fn comment_preview(text: &str) -> String {
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
        (Some(from), Some(to)) => format!(
            "За период с {} по {}",
            from.format("%d.%m.%Y"),
            to.format("%d.%m.%Y")
        ),
        (Some(from), None) => format!("За период с {}", from.format("%d.%m.%Y")),
        (None, Some(to)) => format!("За период по {}", to.format("%d.%m.%Y")),
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
        root.child(self.render_table(cx))
            .child(self.render_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        card_label, cards_for_bank, comment_preview, update_bank_filter_selection, FormValues,
    };
    use chrono::NaiveDate;
    use finance_core::{Currency, PaymentCardType, PaymentDocument, PaymentSystem};
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
        let mut card_filter = Some(10);

        let changed = update_bank_filter_selection(&mut bank_filter, &mut card_filter, Some(2));

        assert!(changed);
        assert_eq!(bank_filter, Some(2));
        assert_eq!(card_filter, None);
    }

    #[test]
    fn form_values_are_unchanged_when_they_match_the_initial_state() {
        let initial = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 20).expect("valid test date")),
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
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
            card_id: None,
            affects_deposit_rate: false,
        };
        let edited = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 20).expect("valid test date")),
            amount: "200".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
            card_id: None,
            affects_deposit_rate: false,
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
            card_id: None,
            affects_deposit_rate: false,
        };
        let edited = FormValues {
            date: Some(NaiveDate::from_ymd_opt(2026, 11, 21).expect("valid test date")),
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
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
            card_id: None,
            affects_deposit_rate: false,
        };
        let edited = FormValues {
            date: None,
            amount: "100".to_owned(),
            comment: "Зарплата".to_owned(),
            currency: None,
            category_id: Some(1),
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
}
