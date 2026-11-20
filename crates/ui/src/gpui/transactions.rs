//! Страницы «Доходы» и «Расходы» на `gpui-kit`. Поведение соответствует
//! `transactions_page.rs`; SQL и финансовые правила остаются в `finance-db` и
//! `finance-core`.

use std::collections::HashSet;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use finance_core::{
    parse_amount_expression, summarize_transaction_balance, Category, Currency, CurrencyDefinition,
    FinancialTransaction, PaymentCardType, PaymentDocument, TransactionBalance, TransactionKind,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IndexPath, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_decimal::Decimal;

use super::support::category_tree_labels;
use super::support::{comment_suggestions, DIVISION_ROUNDING_WARNING};
use super::support::{set_period_dates, DatePeriod};

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

fn parse_optional_date(value: &str) -> Result<Option<NaiveDate>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(value, DATE_FORMAT)
        .map(Some)
        .map_err(|_| "Введите дату фильтра в формате ГГГГ-ММ-ДД".to_owned())
}

fn card_label(card: &PaymentDocument) -> String {
    let card_type = match card.card_type {
        PaymentCardType::Debit => "дебетовая",
        PaymentCardType::Credit => "кредитная",
    };
    format!(
        "{} — {} #{} ({card_type}, {})",
        card.bank_name,
        card.payment_system.code(),
        card.id,
        card.currency.map_or("валюта не указана", Currency::code)
    )
}

/// Карты, доступные для выбора при данной валюте операции.
fn eligible_cards(cards: &[PaymentDocument], currency: Option<Currency>) -> Vec<&PaymentDocument> {
    cards
        .iter()
        .filter(|card| card.currency.is_some() && card.currency == currency)
        .collect()
}

struct Form {
    id: Option<i64>,
    date: Entity<InputState>,
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
}

pub(crate) struct TransactionsPage {
    database: Rc<Database>,
    kind: TransactionKind,
    categories: Vec<Category>,
    currencies: Vec<CurrencyDefinition>,
    cards: Vec<PaymentDocument>,
    comment_history: Vec<String>,
    transactions: Vec<FinancialTransaction>,
    balances: Option<[TransactionBalance; 2]>,
    balance_period: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
    message: Option<(String, bool)>,
    search: Entity<InputState>,
    from: Entity<InputState>,
    to: Entity<InputState>,
    category_filter: Option<i64>,
    category_filter_ids: Vec<i64>,
    category_filter_select: Choice,
    currency_filter: Option<Currency>,
    currency_filter_select: Choice,
    period_select: Choice,
    form: Option<Form>,
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
        let from = cx.new(|cx| InputState::new(window, cx).placeholder("С: ГГГГ-ММ-ДД"));
        let to = cx.new(|cx| InputState::new(window, cx).placeholder("По: ГГГГ-ММ-ДД"));
        let category_filter_select = choice(vec![NONE_LABEL.to_owned()], 0, window, cx);
        let currency_filter_select = choice(Self::currency_filter_labels(), 0, window, cx);
        let period_select = choice(Self::period_labels(), 0, window, cx);

        let mut subscriptions = Vec::new();
        for input in [&search, &from, &to] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.refresh(cx);
                    }
                },
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
            currencies: Vec::new(),
            cards: Vec::new(),
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
            currency_filter: None,
            currency_filter_select,
            period_select,
            form: None,
            _subscriptions: subscriptions,
        };
        page.reload_categories(window, cx);
        page.refresh(cx);
        page
    }

    /// Обновляет категории и список после изменений в справочниках.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Загружает операции и сальдо по текущим фильтрам; ошибки показываются, а не маскируются.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let from = self.from.read(cx).value().to_string();
        let to = self.to.read(cx).value().to_string();
        let search = self.search.read(cx).value().to_string();
        let (from, to) = match (parse_optional_date(&from), parse_optional_date(&to)) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(message), _) | (_, Err(message)) => {
                self.error(message, cx);
                return;
            }
        };
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
        match finance_db::list_transactions(
            &self.database,
            self.kind,
            from,
            to,
            self.category_filter,
            self.currency_filter,
            &search,
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
        let mut from = self.from.read(cx).value().to_string();
        let mut to = self.to.read(cx).value().to_string();
        if !set_period_dates(period, Local::now().date_naive(), &mut from, &mut to) {
            return;
        }
        self.from
            .update(cx, |input, cx| input.set_value(from, window, cx));
        self.to
            .update(cx, |input, cx| input.set_value(to, window, cx));
        self.refresh(cx);
    }

    fn has_active_filters(&self, cx: &App) -> bool {
        !self.search.read(cx).value().is_empty()
            || !self.from.read(cx).value().is_empty()
            || !self.to.read(cx).value().is_empty()
            || self.category_filter.is_some()
            || self.currency_filter.is_some()
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.search, &self.from, &self.to] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.category_filter = None;
        self.currency_filter = None;
        for select in [
            &self.category_filter_select,
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

        let date_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("ГГГГ-ММ-ДД");
            state.set_value(date.format(DATE_FORMAT).to_string(), window, cx);
            state
        });
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
            date: date_input,
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
        });
        self.rebuild_category_select(window, cx);
        self.rebuild_card_select(window, cx);
        cx.notify();
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

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let date_text = form.date.read(cx).value().to_string();
        let amount_text = form.amount.read(cx).value().to_string();
        let comment = form.comment.read(cx).value().to_string();
        let Ok(date) = NaiveDate::parse_from_str(date_text.trim(), DATE_FORMAT) else {
            self.error("Введите дату в формате ГГГГ-ММ-ДД", cx);
            return;
        };
        let amount = match parse_amount_expression(&amount_text) {
            Ok(amount) => amount,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        };
        let Some(category_id) = form.category_id else {
            self.error("Выберите категорию", cx);
            return;
        };
        let Some(currency) = form.currency else {
            self.error("Выберите валюту", cx);
            return;
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
            }
            Err(error) => self.error(error.to_string(), cx),
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

    fn render_filters(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.has_active_filters(cx);
        let this = cx.entity();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(200.0))
                    .child(Input::new(&self.search).cleanable(true)),
            )
            .child(
                div()
                    .w(px(140.0))
                    .child(Select::new(&self.period_select).placeholder("Период")),
            )
            .child(div().w(px(150.0)).child(Input::new(&self.from)))
            .child(div().w(px(150.0)).child(Input::new(&self.to)))
            .child(
                div()
                    .w(px(260.0))
                    .child(Select::new(&self.category_filter_select).placeholder("Категория")),
            )
            .child(
                div()
                    .w(px(110.0))
                    .child(Select::new(&self.currency_filter_select).placeholder("Валюта")),
            )
            .child(
                Button::new("filters-apply")
                    .primary()
                    .label("Применить")
                    .on_click({
                        let this = this.clone();
                        move |_, _, cx| this.update(cx, |page, cx| page.refresh(cx))
                    }),
            )
            .child(
                Button::new("filters-reset")
                    .label("Сбросить фильтры")
                    .disabled(!active)
                    .on_click(cx.listener(|page, _, window, cx| page.reset_filters(window, cx))),
            )
    }

    fn render_form(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let form = self.form.as_ref()?;
        let kind = self.kind;
        let amount_text = form.amount.read(cx).value().to_string();
        let comment_text = form.comment.read(cx).value().to_string();
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let card_missing_currency = form.card_id.is_some()
            && !eligible_cards(&self.cards, form.currency)
                .iter()
                .any(|card| Some(card.id) == form.card_id);
        let rate_eligible = eligible_cards(&self.cards, form.currency)
            .iter()
            .any(|card| Some(card.id) == form.card_id && card.affects_deposit_rate);
        let legacy_debit_expense = form.id.is_some()
            && !form.balance_applied
            && self.cards.iter().any(|card| {
                Some(card.id) == form.card_id && card.card_type == PaymentCardType::Debit
            });

        let total: Option<Result<Decimal, String>> = (!amount_text.trim().is_empty())
            .then(|| parse_amount_expression(&amount_text).map_err(|e| e.to_string()));
        let card_caption = if kind == TransactionKind::Income {
            "Зачислить на карту"
        } else {
            "Оплачено картой"
        };

        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(if form.id.is_some() {
                        "Изменить операцию"
                    } else {
                        "Новая операция"
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child("Дата")
                    .child(div().w(px(140.0)).child(Input::new(&form.date)))
                    .child("Сумма")
                    .child(div().w(px(260.0)).child(Input::new(&form.amount)))
                    .child("Валюта")
                    .child(
                        div()
                            .w(px(100.0))
                            .child(Select::new(&form.currency_select).placeholder("Выберите")),
                    )
                    .child("Категория")
                    .child(
                        div()
                            .w(px(280.0))
                            .child(Select::new(&form.category_select).placeholder("Выберите")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(card_caption)
                    .child(div().w(px(420.0)).child(Select::new(&form.card_select)))
                    .when(card_missing_currency, |row| {
                        row.child(div().text_color(danger).child(
                            if kind == TransactionKind::Income {
                                "Выберите карту в валюте дохода или отмените выбор карты"
                            } else {
                                "Выберите карту в валюте расхода или отмените выбор карты"
                            },
                        ))
                    }),
            );

        if kind == TransactionKind::Income {
            panel = panel.child(div().text_color(muted).child(
                "Доступны дебетовые карты с указанной валютой. Для ранее созданных карт сначала укажите валюту в разделе «Платёжные карты».",
            ));
        } else {
            let enabled = rate_eligible || form.affects_deposit_rate;
            panel = panel
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
                .when(!rate_eligible, |p| {
                    p.child(div().text_color(muted).child(
                        "Для учёта выберите карту с совпадающей валютой и включённым влиянием на ставку.",
                    ))
                })
                .child(div().text_color(muted).child(
                    "Расход с дебетовой карты списывается с её баланса; кредитный лимит не меняется. Оценка процентов пока не рассчитывается.",
                ))
                .when(legacy_debit_expense, |p| {
                    p.child(div().text_color(cx.theme().warning).child(
                        "Этот расход ранее не списывался с баланса. При сохранении будет списана его текущая сумма.",
                    ))
                });
        }

        panel = panel
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child("Комментарий")
                    .child(div().w(px(480.0)).child(Textarea::new(&form.comment))),
            )
            .when(!form.suggestions.is_empty(), |p| {
                let buttons = form.suggestions.iter().enumerate().map(|(index, text)| {
                    let text = text.clone();
                    Button::new(("suggestion", index))
                        .compact()
                        .label(text.clone())
                        .on_click(cx.listener(move |page, _, window, cx| {
                            if let Some(form) = page.form.as_mut() {
                                form.suggestions.clear();
                                let comment = form.comment.clone();
                                comment.update(cx, |state, cx| {
                                    state.set_value(text.clone(), window, cx)
                                });
                            }
                            cx.notify();
                        }))
                });
                p.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child("Ранее использованные комментарии:")
                        .children(buttons),
                )
            })
            .when(amount_text.contains('/'), |p| {
                p.child(div().text_color(muted).child(DIVISION_ROUNDING_WARNING))
            });
        let _ = comment_text;

        Some(
            panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().child(match total {
                        Some(Ok(total)) => div().child(format!("Итого: {total}")),
                        Some(Err(error)) => div().text_color(danger).child(error),
                        None => div(),
                    }))
                    .child(
                        Button::new("form-cancel")
                            .label("Отмена")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.form = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("form-save")
                            .primary()
                            .label("Сохранить")
                            .on_click(cx.listener(|page, _, _, cx| page.save(cx))),
                    ),
            ),
        )
    }

    fn render_table(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let stripe = cx.theme().muted;
        let count = self.transactions.len();
        let is_income = self.kind == TransactionKind::Income;
        let headers = if is_income {
            [
                "Дата",
                "Категория",
                "Сумма",
                "Валюта",
                "Комментарии",
                "Действия",
            ]
        } else {
            [
                "Дата",
                "Сумма",
                "Валюта",
                "Категория",
                "Комментарий",
                "Действия",
            ]
        };
        let header = div()
            .flex()
            .when(is_income, |header| header.w_full())
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_b_1()
            .border_color(border)
            .font_weight(FontWeight::BOLD)
            .children(headers.into_iter().enumerate().map(|(index, title)| {
                if is_income {
                    income_cell(index, title.into())
                } else {
                    cell(COLUMN_WIDTHS[index], title.into())
                }
            }));
        let body = if count == 0 {
            div()
                .when(is_income, |body| body.w_full())
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
            .when(is_income, |body| body.w_full())
            .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(is_income, |table| table.w_full())
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
        let id = transaction.id;
        let is_income = self.kind == TransactionKind::Income;
        let date = transaction.date.format("%d.%m.%Y").to_string();
        let amount = transaction.amount.to_string();
        let currency = transaction.currency.code().to_owned();
        let values = if is_income {
            vec![
                date,
                transaction.category_name.clone(),
                amount,
                currency,
                transaction.comment.clone(),
            ]
        } else {
            vec![
                date,
                amount,
                currency,
                transaction.category_name.clone(),
                transaction.comment.clone(),
            ]
        };
        let edit_target = transaction.clone();
        let delete_target = transaction;
        div()
            .id(("transaction-row", id as usize))
            .flex()
            .when(is_income, |row| row.w_full())
            .h(px(ROW_HEIGHT))
            .items_center()
            .when(index % 2 == 1, |row| row.bg(stripe))
            .children(values.into_iter().enumerate().map(|(column, value)| {
                if is_income {
                    if column == 4 && !value.is_empty() {
                        income_comment_cell(id, value, cx).into_any_element()
                    } else {
                        income_cell(column, value).into_any_element()
                    }
                } else {
                    cell(COLUMN_WIDTHS[column], value).into_any_element()
                }
            }))
            .child(
                div()
                    .w(px(COLUMN_WIDTHS[5]))
                    .flex()
                    .gap_1()
                    .when(is_income, |actions| actions.justify_end())
                    .child(
                        Button::new(("edit", id as usize))
                            .compact()
                            .label("Изменить")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.open_form(Some(&edit_target), window, cx)
                            })),
                    )
                    .child(
                        Button::new(("delete", id as usize))
                            .compact()
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

const COLUMN_WIDTHS: [f32; 6] = [115.0, 135.0, 90.0, 220.0, 320.0, 180.0];
const INCOME_FIXED_COLUMN_WIDTHS: [f32; 4] = [115.0, 220.0, 135.0, 90.0];
const ACTIONS_COLUMN_WIDTH: f32 = 180.0;

fn cell(width: f32, text: String) -> Div {
    div().w(px(width)).px_2().overflow_hidden().child(text)
}

fn income_cell(index: usize, text: String) -> Div {
    match index {
        0..=3 => cell(INCOME_FIXED_COLUMN_WIDTHS[index], text),
        4 => div()
            .flex_1()
            .min_w_0()
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(text),
        _ => div()
            .w(px(ACTIONS_COLUMN_WIDTH))
            .px_2()
            .flex()
            .justify_end()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(text),
    }
}

fn income_comment_cell(id: i64, text: String, cx: &mut Context<TransactionsPage>) -> AnyElement {
    let preview = comment_preview(&text);
    div()
        .id(("income-comment", id as usize))
        .flex_1()
        .min_w_0()
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
        let mut root = div().size_full().flex().flex_col().gap_2().p_3();
        root = root.child(
            div().flex().items_center().gap_2().child(
                Button::new("new-transaction")
                    .primary()
                    .label(match self.kind {
                        TransactionKind::Income => "+ Добавить доход",
                        TransactionKind::Expense => "+ Добавить расход",
                    })
                    .disabled(self.form.is_some())
                    .on_click(cx.listener(|page, _, window, cx| page.open_form(None, window, cx))),
            ),
        );
        root = root.child(self.render_filters(cx));
        if let Some((message, true)) = &self.message {
            root = root.child(div().text_color(danger).child(message.clone()));
        }
        if !has_active_category {
            root = root.child("Сначала добавьте категорию в разделе «Справочники».");
        }
        if let Some(form) = self.render_form(cx) {
            root = root.child(form);
        }
        root.child(self.render_table(cx))
            .child(self.render_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::comment_preview;

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
