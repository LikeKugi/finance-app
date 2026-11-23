//! Страница «Переводы» на `gpui-kit`. Журнал операций — единственный источник
//! истории изменения остатков; SQL и финансовые правила остаются в `finance-db`
//! и `finance-core`.

use std::cmp::Ordering;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use finance_core::{
    parse_amount, Currency, PaymentCardType, PaymentDocument, SavingsAccount, SavingsTerm,
    TransferDetails, TransferEndpoint, TransferOperationType, TransferRecord,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerState};
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
use rust_decimal::Decimal;

use super::helpers::{format_currency, format_date};
use super::helpers::DATE_FORMAT;

const ALL_LABEL: &str = "Все";
const EXTERNAL_LABEL: &str = "Внешние средства";
const CASH_LABEL: &str = "Наличные";
const COLUMN_HEADINGS: [&str; 8] = [
    "Дата",
    "Операция",
    "Откуда",
    "Куда",
    "Сумма",
    "Валюта",
    "Комментарий",
    "Действия",
];
const DEFAULT_COLUMN_WIDTHS: [f32; 8] = [110.0, 140.0, 180.0, 180.0, 110.0, 80.0, 220.0, 160.0];
const COLUMN_WIDTHS_KEY: &str = "table_columns.transfers.v1";
const COL_ACTIONS: usize = 7;
const ACTIONS_COLUMN_WIDTH: f32 = 160.0;
const MIN_COLUMN_WIDTH: f32 = 60.0;
const MAX_COLUMN_WIDTH: f32 = 1200.0;
const FOOTER_NOTE: &str = "Пополнения и снятие наличных не создают доходы и расходы: они изменяют только остатки и журнал переводов. Поэтому сумма остатков может отличаться от отчёта «Анализ».";
const FOOTER_WARNING: &str = "Переводы выполняются только между счетами с явно заданной и совпадающей валютой. Кредитные карты не участвуют: их лимит не является денежными средствами.";
const TYPE_FILTER_ITEM_ID: &str = "transfers-type-filter-item";
const SOURCE_FILTER_ITEM_ID: &str = "transfers-source-filter-item";
const DESTINATION_FILTER_ITEM_ID: &str = "transfers-destination-filter-item";

type Choice = Entity<SelectState<Vec<SharedString>>>;
type TooltipChoice = Entity<SelectState<Vec<TooltipFilterItem>>>;
type EndpointOptions = Vec<(TransferEndpoint, String)>;

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
    cx: &mut App,
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
    cx: &mut App,
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

fn selected_date(state: &DatePickerState) -> Option<NaiveDate> {
    match state.date() {
        Date::Single(Some(date)) => Some(date),
        _ => None,
    }
}

fn currency_index(currency: Currency) -> usize {
    match currency {
        Currency::Rub => 0,
        Currency::Usd => 1,
    }
}

fn currency_labels(first: &str) -> Vec<String> {
    std::iter::once(first.to_owned())
        .chain(
            Currency::ALL
                .into_iter()
                .map(|currency| currency.code().to_owned()),
        )
        .collect()
}

fn eligible_cards(
    cards: &[PaymentDocument],
    currency: Option<Currency>,
) -> Vec<(TransferEndpoint, String)> {
    cards
        .iter()
        .filter(|card| card.card_type == PaymentCardType::Debit)
        .filter(|card| card.currency.is_some())
        .filter(|card| currency.is_none_or(|selected| card.currency == Some(selected)))
        .map(|card| {
            let name = card
                .name
                .clone()
                .unwrap_or_else(|| format!("Карта №{}", card.id));
            (
                TransferEndpoint::Card(card.id),
                format!("{name} · {}", card.bank_name),
            )
        })
        .collect()
}

fn eligible_savings(
    savings: &[SavingsAccount],
    currency: Option<Currency>,
) -> Vec<(TransferEndpoint, String)> {
    savings
        .iter()
        .filter(|account| account.is_active)
        .filter(|account| account.currency.is_some())
        .filter(|account| currency.is_none_or(|selected| account.currency == Some(selected)))
        .map(|account| {
            (
                TransferEndpoint::Savings(account.id),
                format!("{} · {}", account.title, account.bank_name),
            )
        })
        .collect()
}

/// Достаточно ли счетов для создания перевода: кредитные карты и счета
/// без явно заданной валюты не участвуют.
pub(crate) fn can_create_transfer(cards: &[PaymentDocument], savings: &[SavingsAccount]) -> bool {
    !eligible_cards(cards, None).is_empty() || !eligible_savings(savings, None).is_empty()
}

fn endpoint_label(
    endpoint: TransferEndpoint,
    cards: &[PaymentDocument],
    savings: &[SavingsAccount],
) -> String {
    match endpoint {
        TransferEndpoint::External => EXTERNAL_LABEL.to_owned(),
        TransferEndpoint::Cash => CASH_LABEL.to_owned(),
        TransferEndpoint::Card(id) => cards
            .iter()
            .find(|card| card.id == id)
            .map(|card| {
                let name = card
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Карта №{}", card.id));
                format!("{name} · {}", card.bank_name)
            })
            .unwrap_or_else(|| format!("Карта №{id}")),
        TransferEndpoint::Savings(id) => savings
            .iter()
            .find(|account| account.id == id)
            .map(|account| format!("{} · {}", account.title, account.bank_name))
            .unwrap_or_else(|| format!("Накопление №{id}")),
    }
}

/// Чистое представление значений формы; не зависит от GUI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TransferFormValues {
    id: Option<i64>,
    date: String,
    operation_type: Option<TransferOperationType>,
    source: Option<TransferEndpoint>,
    destination: Option<TransferEndpoint>,
    amount: String,
    currency: Option<Currency>,
    comment: String,
}

impl TransferFormValues {
    fn new_for(cards: &[PaymentDocument], savings: &[SavingsAccount]) -> Self {
        let card_options = eligible_cards(cards, None);
        let saving_options = eligible_savings(savings, None);
        let currency = card_options
            .first()
            .and_then(|(endpoint, _)| {
                cards
                    .iter()
                    .find(|card| TransferEndpoint::Card(card.id) == *endpoint)
            })
            .and_then(|card| card.currency)
            .or_else(|| {
                saving_options.first().and_then(|(endpoint, _)| {
                    savings
                        .iter()
                        .find(|account| TransferEndpoint::Savings(account.id) == *endpoint)
                        .and_then(|account| account.currency)
                })
            })
            .unwrap_or(Currency::Rub);
        let mut operation_type = TransferOperationType::Transfer;
        let mut source = card_options
            .first()
            .map(|(endpoint, _)| *endpoint)
            .or_else(|| saving_options.first().map(|(endpoint, _)| *endpoint))
            .unwrap_or(TransferEndpoint::External);
        let mut destination = if source == TransferEndpoint::External {
            TransferEndpoint::Cash
        } else {
            saving_options
                .iter()
                .find(|(endpoint, _)| *endpoint != source)
                .or_else(|| {
                    card_options
                        .iter()
                        .find(|(endpoint, _)| *endpoint != source)
                })
                .map(|(endpoint, _)| *endpoint)
                .unwrap_or(TransferEndpoint::Cash)
        };
        if destination == TransferEndpoint::Cash && source != TransferEndpoint::External {
            operation_type = TransferOperationType::TopUp;
            destination = source;
            source = TransferEndpoint::External;
        }
        Self {
            date: Local::now().date_naive().format(DATE_FORMAT).to_string(),
            operation_type: Some(operation_type),
            source: Some(source),
            destination: Some(destination),
            currency: Some(currency),
            ..Self::default()
        }
    }

    fn from_record(record: &TransferRecord) -> Self {
        Self {
            id: Some(record.id),
            date: record.operation_date.format(DATE_FORMAT).to_string(),
            operation_type: Some(record.operation_type),
            source: Some(record.source),
            destination: Some(record.destination),
            amount: record.amount.to_string(),
            currency: Some(record.currency),
            comment: record.comment.clone().unwrap_or_default(),
        }
    }

    fn details(&self) -> Result<TransferDetails, String> {
        let operation_date = NaiveDate::parse_from_str(&self.date, DATE_FORMAT)
            .map_err(|_| "Введите дату в формате ГГГГ-ММ-ДД".to_owned())?;
        let amount = parse_amount(&self.amount).map_err(|error| error.to_string())?;
        let comment = self.comment.trim();
        let details = TransferDetails {
            operation_date,
            operation_type: self
                .operation_type
                .ok_or_else(|| "Выберите тип операции".to_owned())?,
            source: self.source.ok_or_else(|| "Выберите источник".to_owned())?,
            destination: self
                .destination
                .ok_or_else(|| "Выберите получателя".to_owned())?,
            amount,
            currency: self.currency.ok_or_else(|| "Выберите валюту".to_owned())?,
            comment: (!comment.is_empty()).then(|| comment.to_owned()),
        };
        details.validate().map_err(|error| error.to_string())?;
        Ok(details)
    }
}

/// Итоги остатков по валютам: карты и накопления не складываются между валютами.
fn balance_summary(
    cards: &[PaymentDocument],
    savings: &[SavingsAccount],
) -> Result<[(Decimal, Decimal); 2], String> {
    let mut totals = [(Decimal::ZERO, Decimal::ZERO); 2];
    for card in cards {
        if card.card_type != PaymentCardType::Debit {
            continue;
        }
        if let Some(currency) = card.currency {
            let index = currency_index(currency);
            totals[index].0 = totals[index]
                .0
                .checked_add(card.amount)
                .ok_or_else(|| "Не удалось рассчитать итог по картам: переполнение".to_owned())?;
        }
    }
    for account in savings {
        if let Some(currency) = account.currency {
            let index = currency_index(currency);
            totals[index].1 = totals[index].1.checked_add(account.amount).ok_or_else(|| {
                "Не удалось рассчитать итог по накоплениям: переполнение".to_owned()
            })?;
        }
    }
    Ok(totals)
}

fn endpoint_options(
    operation_type: TransferOperationType,
    cards: &[PaymentDocument],
    savings: &[SavingsAccount],
    currency: Option<Currency>,
) -> (EndpointOptions, EndpointOptions) {
    let mut accounts = eligible_cards(cards, currency);
    accounts.extend(eligible_savings(savings, currency));
    match operation_type {
        TransferOperationType::TopUp => (
            vec![(TransferEndpoint::External, EXTERNAL_LABEL.to_owned())],
            accounts,
        ),
        TransferOperationType::Transfer => (accounts.clone(), accounts),
        TransferOperationType::Withdrawal => (
            accounts,
            vec![(TransferEndpoint::Cash, CASH_LABEL.to_owned())],
        ),
    }
}

/// Состояние открытой формы перевода.
struct Form {
    id: Option<i64>,
    date: Entity<DatePickerState>,
    amount: Entity<InputState>,
    comment: Entity<InputState>,
    type_select: Choice,
    source_select: Choice,
    destination_select: Choice,
    currency_select: Choice,
    operation_type: Option<TransferOperationType>,
    currency: Option<Currency>,
    source: Option<TransferEndpoint>,
    destination: Option<TransferEndpoint>,
    source_options: Vec<TransferEndpoint>,
    destination_options: Vec<TransferEndpoint>,
    error: Option<String>,
    snapshot: TransferFormValues,
    _subscriptions: Vec<Subscription>,
}

pub(crate) struct TransfersPage {
    database: Rc<Database>,
    cards: Vec<PaymentDocument>,
    savings: Vec<SavingsAccount>,
    entries: Vec<TransferRecord>,
    table: Entity<TableState<TransfersTable>>,
    message: Option<(String, bool)>,
    search: Entity<InputState>,
    date_from: Entity<DatePickerState>,
    date_to: Entity<DatePickerState>,
    type_filter: Option<TransferOperationType>,
    source_filter: Option<TransferEndpoint>,
    destination_filter: Option<TransferEndpoint>,
    currency_filter: Option<Currency>,
    type_filter_select: TooltipChoice,
    source_filter_select: TooltipChoice,
    destination_filter_select: TooltipChoice,
    currency_filter_select: Choice,
    source_filter_options: Vec<Option<TransferEndpoint>>,
    destination_filter_options: Vec<Option<TransferEndpoint>>,
    form: Option<Form>,
    confirm_discard: bool,
    _subscriptions: Vec<Subscription>,
}

fn filter_endpoint_options(
    cards: &[PaymentDocument],
    savings: &[SavingsAccount],
) -> Vec<(Option<TransferEndpoint>, String)> {
    let mut options = vec![
        (None, ALL_LABEL.to_owned()),
        (Some(TransferEndpoint::External), EXTERNAL_LABEL.to_owned()),
    ];
    options.extend(cards.iter().map(|card| {
        let name = card
            .name
            .clone()
            .unwrap_or_else(|| format!("Карта №{}", card.id));
        (
            Some(TransferEndpoint::Card(card.id)),
            format!("{name} · {}", card.bank_name),
        )
    }));
    options.extend(savings.iter().map(|account| {
        (
            Some(TransferEndpoint::Savings(account.id)),
            format!("{} · {}", account.title, account.bank_name),
        )
    }));
    options.push((Some(TransferEndpoint::Cash), CASH_LABEL.to_owned()));
    options
}

impl TransfersPage {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Комментарий или счёт"));
        let date_from = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let date_to = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let type_items: Vec<String> = std::iter::once(ALL_LABEL.to_owned())
            .chain(
                TransferOperationType::ALL
                    .into_iter()
                    .map(|item| item.title().to_owned()),
            )
            .collect();
        let type_filter_select =
            tooltip_choice(type_items, Some(0), TYPE_FILTER_ITEM_ID, window, cx);
        let source_filter_select = tooltip_choice(
            vec![ALL_LABEL.to_owned()],
            Some(0),
            SOURCE_FILTER_ITEM_ID,
            window,
            cx,
        );
        let destination_filter_select = tooltip_choice(
            vec![ALL_LABEL.to_owned()],
            Some(0),
            DESTINATION_FILTER_ITEM_ID,
            window,
            cx,
        );
        let currency_filter_select = choice(currency_labels(ALL_LABEL), Some(0), window, cx);

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
                &type_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<TooltipFilterItem>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.type_filter = value.as_deref().and_then(|label| {
                        TransferOperationType::ALL
                            .into_iter()
                            .find(|item| item.title() == label)
                    });
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &source_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<TooltipFilterItem>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.source_filter = value.as_deref().and_then(|label| {
                        let options = filter_endpoint_options(&this.cards, &this.savings);
                        options
                            .into_iter()
                            .find(|(_, item)| item.as_str() == label)
                            .and_then(|(endpoint, _)| endpoint)
                    });
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &destination_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<TooltipFilterItem>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.destination_filter = value.as_deref().and_then(|label| {
                        let options = filter_endpoint_options(&this.cards, &this.savings);
                        options
                            .into_iter()
                            .find(|(_, item)| item.as_str() == label)
                            .and_then(|(endpoint, _)| endpoint)
                    });
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &currency_filter_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.currency_filter = value.as_ref().and_then(|label| match label.as_ref() {
                        "RUB" => Some(Currency::Rub),
                        "USD" => Some(Currency::Usd),
                        _ => None,
                    });
                    cx.notify();
                },
            ),
        ];
        subscriptions.shrink_to_fit();

        let widths = load_column_widths(&database);
        let table_delegate = TransfersTable::new(cx.entity().downgrade(), widths);
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
            cards: Vec::new(),
            savings: Vec::new(),
            entries: Vec::new(),
            table,
            message: None,
            search,
            date_from,
            date_to,
            type_filter: None,
            source_filter: None,
            destination_filter: None,
            currency_filter: None,
            type_filter_select,
            source_filter_select,
            destination_filter_select,
            currency_filter_select,
            source_filter_options: Vec::new(),
            destination_filter_options: Vec::new(),
            form: None,
            confirm_discard: false,
            _subscriptions: subscriptions,
        };
        page.reload(window, cx);
        page
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some((message.into(), true));
        cx.notify();
    }

    fn save_column_widths(&mut self, widths: &[Pixels], cx: &mut Context<Self>) {
        let mut widths: Vec<f32> = widths.iter().map(|width| f32::from(*width)).collect();
        if let Some(width) = widths.get_mut(COL_ACTIONS) {
            *width = ACTIONS_COLUMN_WIDTH;
        }
        self.table
            .update(cx, |table, _| table.delegate_mut().set_widths(&widths));
        if let Err(error) = finance_db::set_app_setting(
            &self.database,
            COLUMN_WIDTHS_KEY,
            &format_column_widths(&widths),
        ) {
            self.error(format!("Не удалось сохранить ширину колонок: {error}"), cx);
        }
    }

    /// Перезагружает справочники и журнал по текущим фильтрам.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_accounts(window, cx);
        self.refresh(cx);
    }

    fn reload_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match finance_db::list_payment_documents(&self.database, None, None, None) {
            Ok(cards) => self.cards = cards,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        }
        match finance_db::list_savings(&self.database, None, None, "") {
            Ok(savings) => self.savings = savings,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        }

        let source_options = filter_endpoint_options(&self.cards, &self.savings);
        let destination_options = source_options.clone();
        self.source_filter_options = source_options
            .iter()
            .map(|(endpoint, _)| *endpoint)
            .collect();
        self.destination_filter_options = destination_options
            .iter()
            .map(|(endpoint, _)| *endpoint)
            .collect();
        if self.source_filter.is_some() && !self.source_filter_options.contains(&self.source_filter)
        {
            self.source_filter = None;
        }
        if self.destination_filter.is_some()
            && !self
                .destination_filter_options
                .contains(&self.destination_filter)
        {
            self.destination_filter = None;
        }

        let mut set_endpoint_items = |select: &TooltipChoice,
                                      options: &[(Option<TransferEndpoint>, String)],
                                      selected: Option<TransferEndpoint>,
                                      item_id: &'static str| {
            let labels: Vec<String> = options.iter().map(|(_, label)| label.clone()).collect();
            let items = tooltip_items(labels, item_id);
            let row = selected
                .and_then(|endpoint| {
                    options
                        .iter()
                        .position(|(option, _)| *option == Some(endpoint))
                })
                .unwrap_or(0);
            select.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                state.set_selected_index(Some(IndexPath::default().row(row)), window, cx);
            });
        };
        set_endpoint_items(
            &self.source_filter_select,
            &source_options,
            self.source_filter,
            SOURCE_FILTER_ITEM_ID,
        );
        set_endpoint_items(
            &self.destination_filter_select,
            &destination_options,
            self.destination_filter,
            DESTINATION_FILTER_ITEM_ID,
        );
        self.rebuild_form_endpoints(window, cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) -> bool {
        let search = self.search.read(cx).value().to_string();
        let from = selected_date(self.date_from.read(cx));
        let to = selected_date(self.date_to.read(cx));
        match finance_db::list_transfers_filtered(
            &self.database,
            from,
            to,
            self.type_filter,
            self.source_filter,
            self.destination_filter,
            self.currency_filter,
            &search,
        ) {
            Ok(entries) => {
                self.table.update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .set_rows(entries.clone(), &self.cards, &self.savings);
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
            || selected_date(self.date_from.read(cx)).is_some()
            || selected_date(self.date_to.read(cx)).is_some()
            || self.type_filter.is_some()
            || self.source_filter.is_some()
            || self.destination_filter.is_some()
            || self.currency_filter.is_some()
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        for picker in [&self.date_from, &self.date_to] {
            picker.update(cx, |state, cx| {
                state.set_date(Date::Single(None), window, cx)
            });
        }
        self.type_filter = None;
        self.source_filter = None;
        self.destination_filter = None;
        self.currency_filter = None;
        for select in [
            &self.type_filter_select,
            &self.source_filter_select,
            &self.destination_filter_select,
        ] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
            });
        }
        self.currency_filter_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        self.apply_filters(cx);
    }

    fn rebuild_form_endpoints(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (operation_type, currency, source, destination) = {
            let Some(form) = self.form.as_ref() else {
                return;
            };
            (
                form.operation_type
                    .unwrap_or(TransferOperationType::Transfer),
                form.currency,
                form.source,
                form.destination,
            )
        };
        let (source_options, destination_options) =
            endpoint_options(operation_type, &self.cards, &self.savings, currency);
        let source_endpoints: Vec<TransferEndpoint> = source_options
            .iter()
            .map(|(endpoint, _)| *endpoint)
            .collect();
        let destination_endpoints: Vec<TransferEndpoint> = destination_options
            .iter()
            .map(|(endpoint, _)| *endpoint)
            .collect();
        let source_labels: Vec<SharedString> = source_options
            .iter()
            .map(|(_, label)| SharedString::from(label.clone()))
            .collect();
        let destination_labels: Vec<SharedString> = destination_options
            .iter()
            .map(|(_, label)| SharedString::from(label.clone()))
            .collect();
        let source_selected = source
            .and_then(|value| {
                source_endpoints
                    .iter()
                    .position(|endpoint| *endpoint == value)
            })
            .or_else(|| (!source_endpoints.is_empty()).then_some(0));
        let destination_selected = destination
            .and_then(|value| {
                destination_endpoints
                    .iter()
                    .position(|endpoint| *endpoint == value)
            })
            .or_else(|| (!destination_endpoints.is_empty()).then_some(0));

        let Some(form) = self.form.as_mut() else {
            return;
        };
        form.source_options = source_endpoints;
        form.destination_options = destination_endpoints;
        form.source = source_selected.and_then(|row| form.source_options.get(row).copied());
        form.destination =
            destination_selected.and_then(|row| form.destination_options.get(row).copied());
        form.source_select.update(cx, |state, cx| {
            state.set_items(source_labels, window, cx);
            state.set_selected_index(
                source_selected.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            );
        });
        form.destination_select.update(cx, |state, cx| {
            state.set_items(destination_labels, window, cx);
            state.set_selected_index(
                destination_selected.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            );
        });
    }

    fn begin_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let initial = TransferFormValues::new_for(&self.cards, &self.savings);
        self.open_form(initial, window, cx);
    }

    fn begin_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.entries.iter().find(|record| record.id == id) else {
            return;
        };
        let initial = TransferFormValues::from_record(record);
        self.open_form(initial, window, cx);
    }

    fn open_form(
        &mut self,
        initial: TransferFormValues,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let date = NaiveDate::parse_from_str(&initial.date, DATE_FORMAT).ok();
        let date_picker = cx.new(|cx| {
            let mut state = DatePickerState::new(window, cx).date_format(DATE_FORMAT);
            state.set_date(Date::Single(date), window, cx);
            state
        });
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
        let amount = text_input(&initial.amount, "Например, 1000.50", window, cx);
        let comment = text_input(&initial.comment, "Необязательно", window, cx);

        let operation_type = initial
            .operation_type
            .unwrap_or(TransferOperationType::Transfer);
        let type_items: Vec<String> = TransferOperationType::ALL
            .into_iter()
            .map(|item| item.title().to_owned())
            .collect();
        let type_selected = TransferOperationType::ALL
            .into_iter()
            .position(|item| item == operation_type);
        let type_select = choice(type_items, type_selected, window, cx);
        let currency = initial.currency.unwrap_or(Currency::Rub);
        let currency_items = currency_labels("");
        let currency_selected = Currency::ALL.into_iter().position(|item| item == currency);
        let currency_select = choice(currency_items, currency_selected, window, cx);
        let source_select = choice(vec![], None, window, cx);
        let destination_select = choice(vec![], None, window, cx);

        let mut subscriptions = vec![
            cx.subscribe_in(
                &type_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let Some(label) = value.as_ref() else {
                        return;
                    };
                    let Some(operation) = TransferOperationType::ALL
                        .into_iter()
                        .find(|item| item.title() == label.as_ref())
                    else {
                        return;
                    };
                    if let Some(form) = this.form.as_mut() {
                        form.operation_type = Some(operation);
                    }
                    this.rebuild_form_endpoints(window, cx);
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &currency_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let selected = value.as_ref().and_then(|label| match label.as_ref() {
                        "RUB" => Some(Currency::Rub),
                        "USD" => Some(Currency::Usd),
                        _ => None,
                    });
                    let Some(selected) = selected else {
                        return;
                    };
                    if let Some(form) = this.form.as_mut() {
                        form.currency = Some(selected);
                    }
                    this.rebuild_form_endpoints(window, cx);
                    cx.notify();
                },
            ),
        ];
        subscriptions.push(cx.subscribe_in(
            &source_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                let Some(label) = value.as_ref() else {
                    return;
                };
                let endpoint = this.form.as_ref().and_then(|form| {
                    form.source_options.iter().copied().find(|endpoint| {
                        endpoint_label(*endpoint, &this.cards, &this.savings) == label.as_ref()
                    })
                });
                if let Some(form) = this.form.as_mut() {
                    form.source = endpoint;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &destination_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                let Some(label) = value.as_ref() else {
                    return;
                };
                let endpoint = this.form.as_ref().and_then(|form| {
                    form.destination_options.iter().copied().find(|endpoint| {
                        endpoint_label(*endpoint, &this.cards, &this.savings) == label.as_ref()
                    })
                });
                if let Some(form) = this.form.as_mut() {
                    form.destination = endpoint;
                }
                cx.notify();
            },
        ));

        self.form = Some(Form {
            id: initial.id,
            date: date_picker,
            amount,
            comment,
            type_select,
            source_select,
            destination_select,
            currency_select,
            operation_type: Some(operation_type),
            currency: Some(currency),
            source: initial.source,
            destination: initial.destination,
            source_options: Vec::new(),
            destination_options: Vec::new(),
            error: None,
            snapshot: initial.clone(),
            _subscriptions: subscriptions,
        });
        self.confirm_discard = false;
        self.rebuild_form_endpoints(window, cx);
        self.open_form_dialog(window, cx);
    }

    fn open_form_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        window.open_dialog(cx, move |dialog: Dialog, _, cx| {
            let (title, body) = this.update(cx, |page, cx| {
                let title = match page.form.as_ref().map(|form| form.id.is_some()) {
                    Some(true) => "Изменение перевода",
                    _ => "Новый перевод",
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

    fn read_form(&self, cx: &App) -> Option<TransferFormValues> {
        let form = self.form.as_ref()?;
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        Some(TransferFormValues {
            id: form.id,
            date: selected_date(form.date.read(cx))
                .map(|date| date.format(DATE_FORMAT).to_string())
                .unwrap_or_default(),
            operation_type: form.operation_type,
            source: form.source,
            destination: form.destination,
            amount: text(&form.amount),
            currency: form.currency,
            comment: text(&form.comment),
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
        match finance_db::save_transfer(&self.database, values.id, &details) {
            Ok(_) => {
                self.form = None;
                if self.refresh(cx) {
                    self.message = Some(("Перевод сохранён".to_owned(), false));
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
                        Button::new("transfers-continue-editing")
                            .label("Вернуться к форме")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.confirm_discard = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("transfers-discard-changes")
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

    fn confirm_delete(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.entries.iter().find(|record| record.id == id).cloned() else {
            return;
        };
        let description = format!(
            "Удалить перевод от {} на {} {} ({} → {})? Остатки будут возвращены к предыдущему состоянию.",
            format_date(record.operation_date),
            record.amount,
            record.currency.code(),
            endpoint_label(record.source, &self.cards, &self.savings),
            endpoint_label(record.destination, &self.cards, &self.savings),
        );
        let this = cx.entity();
        window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
            let this = this.clone();
            alert
                .title("Подтверждение удаления перевода")
                .description(description.clone())
                .ok_text("Удалить")
                .cancel_text("Отмена")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |page, cx| {
                        match finance_db::delete_transfer(&page.database, id) {
                            Ok(()) => {
                                if page.refresh(cx) {
                                    page.message = Some(("Перевод удалён".to_owned(), false));
                                }
                            }
                            Err(error) => page.error(error.to_string(), cx),
                        }
                    });
                    true
                })
        });
    }

    fn clear_button(
        id: &'static str,
        tooltip: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(id)
            .compact()
            .small()
            .label("×")
            .tooltip(tooltip)
            .on_click(cx.listener(move |page, _, window, cx| on_click(page, window, cx)))
    }

    fn render_filters(&mut self, cx: &mut Context<Self>) -> Div {
        let active = self.has_active_filters(cx);
        let from_set = selected_date(self.date_from.read(cx)).is_some();
        let to_set = selected_date(self.date_to.read(cx)).is_some();
        let type_set = self.type_filter.is_some();
        let source_set = self.source_filter.is_some();
        let destination_set = self.destination_filter.is_some();
        let currency_set = self.currency_filter.is_some();
        let search_set = !self.search.read(cx).value().is_empty();
        let filter =
            |label: &'static str, width: f32, clear: Option<AnyElement>, control: AnyElement| {
                div()
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(18.0))
                            .child(div().text_size(px(12.0)).child(label))
                            .children(clear),
                    )
                    .child(control)
            };
        let clear = |element: Button| element.into_any_element();
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
                        search_set.then(|| {
                            clear(Self::clear_button(
                                "clear-transfer-search",
                                "Очистить поиск",
                                cx,
                                |page, window, cx| {
                                    page.search
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    page.apply_filters(cx);
                                },
                            ))
                        }),
                        Input::new(&self.search)
                            .cleanable(true)
                            .small()
                            .suffix(Icon::new(IconName::Search).small())
                            .into_any_element(),
                    ))
                    .child(filter(
                        "С",
                        130.0,
                        from_set.then(|| {
                            clear(Self::clear_button(
                                "clear-transfer-from",
                                "Очистить дату «С»",
                                cx,
                                |page, window, cx| {
                                    page.date_from.update(cx, |state, cx| {
                                        state.set_date(Date::Single(None), window, cx)
                                    });
                                    page.apply_filters(cx);
                                },
                            ))
                        }),
                        DatePicker::new(&self.date_from)
                            .placeholder("Выбрать дату")
                            .cleanable(true)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "По",
                        130.0,
                        to_set.then(|| {
                            clear(Self::clear_button(
                                "clear-transfer-to",
                                "Очистить дату «По»",
                                cx,
                                |page, window, cx| {
                                    page.date_to.update(cx, |state, cx| {
                                        state.set_date(Date::Single(None), window, cx)
                                    });
                                    page.apply_filters(cx);
                                },
                            ))
                        }),
                        DatePicker::new(&self.date_to)
                            .placeholder("Выбрать дату")
                            .cleanable(true)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Тип операции",
                        150.0,
                        type_set.then(|| {
                            clear(Self::clear_button(
                                "clear-transfer-type",
                                "Очистить тип операции",
                                cx,
                                |page, window, cx| {
                                    page.type_filter = None;
                                    page.type_filter_select.update(cx, |state, cx| {
                                        state.set_selected_index(
                                            Some(IndexPath::default().row(0)),
                                            window,
                                            cx,
                                        )
                                    });
                                    page.apply_filters(cx);
                                },
                            ))
                        }),
                        Select::new(&self.type_filter_select)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Источник",
                        180.0,
                        source_set.then(|| {
                            clear(Self::clear_button(
                                "clear-transfer-source",
                                "Очистить источник",
                                cx,
                                |page, window, cx| {
                                    page.source_filter = None;
                                    page.source_filter_select.update(cx, |state, cx| {
                                        state.set_selected_index(
                                            Some(IndexPath::default().row(0)),
                                            window,
                                            cx,
                                        )
                                    });
                                    page.apply_filters(cx);
                                },
                            ))
                        }),
                        Select::new(&self.source_filter_select)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Назначение",
                        180.0,
                        destination_set.then(|| {
                            clear(Self::clear_button(
                                "clear-transfer-destination",
                                "Очистить назначение",
                                cx,
                                |page, window, cx| {
                                    page.destination_filter = None;
                                    page.destination_filter_select.update(cx, |state, cx| {
                                        state.set_selected_index(
                                            Some(IndexPath::default().row(0)),
                                            window,
                                            cx,
                                        )
                                    });
                                    page.apply_filters(cx);
                                },
                            ))
                        }),
                        Select::new(&self.destination_filter_select)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Валюта",
                        110.0,
                        currency_set.then(|| {
                            clear(Self::clear_button(
                                "clear-transfer-currency",
                                "Очистить валюту",
                                cx,
                                |page, window, cx| {
                                    page.currency_filter = None;
                                    page.currency_filter_select.update(cx, |state, cx| {
                                        state.set_selected_index(
                                            Some(IndexPath::default().row(0)),
                                            window,
                                            cx,
                                        )
                                    });
                                    page.apply_filters(cx);
                                },
                            ))
                        }),
                        Select::new(&self.currency_filter_select)
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
                        Button::new("transfers-apply-filters")
                            .primary()
                            .small()
                            .label("Применить")
                            .on_click(cx.listener(|page, _, _, cx| page.apply_filters(cx))),
                    )
                    .child(
                        Button::new("transfers-reset-filters")
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
        let warning_color = cx.theme().warning;
        let operation = form
            .operation_type
            .unwrap_or(TransferOperationType::Transfer);
        let form_date = selected_date(form.date.read(cx));
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
        let fields = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                row()
                    .child(labelled(
                        "Дата",
                        140.0,
                        DatePicker::new(&form.date)
                            .placeholder("Выбрать дату")
                            .into_any_element(),
                    ))
                    .child(labelled(
                        "Тип операции",
                        180.0,
                        Select::new(&form.type_select).into_any_element(),
                    ))
                    .child(labelled(
                        "Валюта",
                        120.0,
                        Select::new(&form.currency_select).into_any_element(),
                    )),
            )
            .child(
                row()
                    .child(labelled(
                        "Откуда",
                        280.0,
                        Select::new(&form.source_select).into_any_element(),
                    ))
                    .child(labelled(
                        "Куда",
                        280.0,
                        Select::new(&form.destination_select).into_any_element(),
                    )),
            )
            .child(row().child(labelled(
                "Сумма",
                220.0,
                Input::new(&form.amount).into_any_element(),
            )))
            .child(row().child(labelled(
                "Комментарий",
                500.0,
                Input::new(&form.comment).into_any_element(),
            )));

        let early_withdrawal = form
            .source
            .and_then(|endpoint| match endpoint {
                TransferEndpoint::Savings(id) => self
                    .savings
                    .iter()
                    .find(|account| account.id == id)
                    .and_then(|account| {
                        if account.term_type == SavingsTerm::Limited {
                            account
                                .term_end_date
                                .and_then(|end| form_date.map(|date| date < end))
                        } else {
                            None
                        }
                    }),
                _ => None,
            })
            .unwrap_or(false);

        let mut panel = div().flex().flex_col().gap_3().p_4().child(fields);
        let operation_warning = match operation {
            TransferOperationType::TopUp => {
                Some("Запись дохода не создаётся: изменяется только остаток выбранного счёта.")
            }
            TransferOperationType::Withdrawal => {
                Some("Деньги выводятся из учёта остатков; запись расхода не создаётся.")
            }
            TransferOperationType::Transfer => None,
        };
        if let Some(warning) = operation_warning {
            panel = panel.child(div().text_color(warning_color).child(warning));
        }
        if early_withdrawal {
            panel = panel.child(div().text_color(warning_color).child(
                "Досрочное изъятие со срочного вклада: банк может применить другие условия.",
            ));
        }
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
                        Button::new("transfers-form-save")
                            .primary()
                            .label("Сохранить")
                            .on_click(cx.listener(|page, _, window, cx| {
                                if page.save(cx) {
                                    page.confirm_discard = false;
                                    window.close_dialog(cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("transfers-form-cancel")
                            .label("Отмена")
                            .on_click(
                                cx.listener(|page, _, window, cx| page.request_cancel(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_summary(&self, _: &Context<Self>) -> Div {
        let text = match balance_summary(&self.cards, &self.savings) {
            Ok(totals) => totals
                .into_iter()
                .zip(Currency::ALL)
                .map(|((cards, savings), currency)| {
                    let total = cards
                        .checked_add(savings)
                        .map(|value| format_currency(&value, 2))
                        .unwrap_or_else(|| "ошибка расчёта".to_owned());
                    format!(
                        "{}: карты — {}, накопления — {}, вместе — {}",
                        currency.code(),
                        format_currency(&cards, 2),
                        format_currency(&savings, 2),
                        total
                    )
                })
                .collect::<Vec<_>>()
                .join(" · "),
            Err(error) => error,
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child("Остатки:")
            .child(text)
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

fn load_column_widths(database: &Database) -> Vec<f32> {
    match finance_db::get_app_setting(database, COLUMN_WIDTHS_KEY) {
        Ok(Some(value)) => parse_column_widths(&value).unwrap_or_else(|| {
            eprintln!("Некорректная сохранённая ширина колонок переводов; использованы значения по умолчанию");
            DEFAULT_COLUMN_WIDTHS.to_vec()
        }),
        Ok(None) => DEFAULT_COLUMN_WIDTHS.to_vec(),
        Err(error) => {
            eprintln!("Не удалось прочитать ширину колонок переводов: {error}");
            DEFAULT_COLUMN_WIDTHS.to_vec()
        }
    }
}

fn endpoint_order(endpoint: TransferEndpoint) -> (u8, i64) {
    match endpoint {
        TransferEndpoint::External => (0, 0),
        TransferEndpoint::Cash => (1, 0),
        TransferEndpoint::Card(id) => (2, id),
        TransferEndpoint::Savings(id) => (3, id),
    }
}

fn default_order(left: &TransferRow, right: &TransferRow) -> Ordering {
    left.record.id.cmp(&right.record.id)
}

fn sort_transfers(rows: &mut [TransferRow], sort: Option<(usize, ColumnSort)>) {
    let Some((column, direction)) = sort else {
        rows.sort_by(default_order);
        return;
    };
    rows.sort_by(|left, right| {
        let ordering = match column {
            0 => left.record.operation_date.cmp(&right.record.operation_date),
            1 => left
                .record
                .operation_type
                .code()
                .cmp(right.record.operation_type.code()),
            2 => endpoint_order(left.record.source).cmp(&endpoint_order(right.record.source)),
            3 => endpoint_order(left.record.destination)
                .cmp(&endpoint_order(right.record.destination)),
            4 => left.record.amount.cmp(&right.record.amount),
            5 => left
                .record
                .currency
                .code()
                .cmp(right.record.currency.code()),
            6 => left
                .comment
                .to_lowercase()
                .cmp(&right.comment.to_lowercase()),
            _ => Ordering::Equal,
        };
        let ordering = if direction == ColumnSort::Descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| default_order(left, right))
    });
}

struct TransferRow {
    record: TransferRecord,
    source_label: String,
    destination_label: String,
    comment: String,
}

struct TransfersTable {
    page: WeakEntity<TransfersPage>,
    rows: Vec<TransferRow>,
    widths: Vec<f32>,
    sort: Option<(usize, ColumnSort)>,
}

impl TransfersTable {
    fn new(page: WeakEntity<TransfersPage>, widths: Vec<f32>) -> Self {
        Self {
            page,
            rows: Vec::new(),
            widths,
            sort: None,
        }
    }

    fn set_rows(
        &mut self,
        records: Vec<TransferRecord>,
        cards: &[PaymentDocument],
        savings: &[SavingsAccount],
    ) {
        self.rows = records
            .into_iter()
            .map(|record| TransferRow {
                source_label: endpoint_label(record.source, cards, savings),
                destination_label: endpoint_label(record.destination, cards, savings),
                comment: record.comment.clone().unwrap_or_default(),
                record,
            })
            .collect();
        sort_transfers(&mut self.rows, self.sort);
    }

    fn set_widths(&mut self, widths: &[f32]) {
        if widths.len() == self.widths.len() {
            self.widths = widths.to_vec();
        }
    }
}

impl TableDelegate for TransfersTable {
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
        if col_ix < COL_ACTIONS {
            column.sort(match self.sort {
                Some((sorted, direction)) if sorted == col_ix => direction,
                _ => ColumnSort::Default,
            })
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
        sort_transfers(&mut self.rows, self.sort);
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
            .child("Переводов по выбранным условиям нет.")
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row_ix];
        let content = match col_ix {
            0 => div()
                .child(format_date(row.record.operation_date))
                .into_any_element(),
            1 => div()
                .child(row.record.operation_type.title().to_owned())
                .into_any_element(),
            2 => div().child(row.source_label.clone()).into_any_element(),
            3 => div()
                .child(row.destination_label.clone())
                .into_any_element(),
            4 => div()
                .w_full()
                .text_right()
                .child(format_currency(&row.record.amount, 2))
                .into_any_element(),
            5 => div()
                .child(row.record.currency.code().to_owned())
                .into_any_element(),
            6 => {
                let text: String = row.comment.chars().take(40).collect();
                let text = if row.comment.chars().count() > 40 {
                    format!("{text}…")
                } else {
                    text
                };
                div().child(text).into_any_element()
            }
            COL_ACTIONS => {
                let id = row.record.id;
                let edit_page = self.page.clone();
                let delete_page = self.page.clone();
                div()
                    .flex()
                    .w_full()
                    .justify_center()
                    .gap_1()
                    .child(
                        Button::new(("edit-transfer", id as usize))
                            .xsmall()
                            .label("Изменить")
                            .on_click(move |_, window, cx| {
                                let _ = edit_page
                                    .update(cx, |page, cx| page.begin_edit(id, window, cx));
                            }),
                    )
                    .child(
                        Button::new(("delete-transfer", id as usize))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .on_click(move |_, window, cx| {
                                let _ = delete_page
                                    .update(cx, |page, cx| page.confirm_delete(id, window, cx));
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

impl Render for TransfersPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let success = cx.theme().success;
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let has_accounts = can_create_transfer(&self.cards, &self.savings);
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
                        .child("Переводы"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("new-transfer")
                                .primary()
                                .large()
                                .label("Добавить перевод")
                                .icon(IconName::Plus)
                                .disabled(self.form.is_some() || !has_accounts)
                                .on_click(
                                    cx.listener(|page, _, window, cx| page.begin_new(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("refresh-transfers")
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
        root = root.child(self.render_summary(cx));
        if !has_accounts {
            root = root.child(div().text_color(warning).child(
                "Сначала добавьте дебетовую карту или накопление с указанной валютой в разделе «Справочники» и «Платёжные карты».",
            ));
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
    use finance_core::{
        InterestDestination, PaymentSystem, TransferRecord, MAX_TRANSFER_COMMENT_LENGTH,
    };
    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::*;

    fn card(id: i64, card_type: PaymentCardType, currency: Option<Currency>) -> PaymentDocument {
        PaymentDocument {
            id,
            name: Some(format!("Карта {id}")),
            bank_id: 1,
            bank_name: "Банк".to_owned(),
            payment_system: PaymentSystem::Visa,
            card_type,
            amount: Decimal::from_str(&format!("{id}00")).unwrap(),
            currency,
            due_date: None,
            grace_period_days: None,
            next_payment_amount: None,
            affects_deposit_rate: false,
            min_monthly_spend: None,
            linked_savings: Vec::new(),
        }
    }

    fn account(
        id: i64,
        currency: Option<Currency>,
        is_active: bool,
        term_type: SavingsTerm,
    ) -> SavingsAccount {
        SavingsAccount {
            id,
            bank_id: 1,
            bank_name: "Банк".to_owned(),
            title: format!("Вклад {id}"),
            amount: Decimal::from_str(&format!("{id}0")).unwrap(),
            currency,
            base_rate: Decimal::ZERO,
            rate_after_threshold: None,
            threshold_amount: None,
            opened_date: None,
            interest_accrual_date: None,
            term_type,
            term_end_date: None,
            interest_destination: InterestDestination::Reinvest,
            is_active,
        }
    }

    #[::core::prelude::v1::test]
    fn transfer_filter_tooltip_items_keep_labels_and_have_unique_ids() {
        let labels = vec![
            ALL_LABEL.to_owned(),
            EXTERNAL_LABEL.to_owned(),
            CASH_LABEL.to_owned(),
        ];

        for item_id in [
            TYPE_FILTER_ITEM_ID,
            SOURCE_FILTER_ITEM_ID,
            DESTINATION_FILTER_ITEM_ID,
        ] {
            let items = tooltip_items(labels.clone(), item_id);

            assert_eq!(
                items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>(),
                labels.iter().map(String::as_str).collect::<Vec<_>>()
            );
            assert_eq!(
                items.iter().map(|item| item.id).collect::<Vec<_>>(),
                [0, 1, 2]
            );
            assert!(items.iter().all(|item| item.item_id == item_id));
            assert_eq!(items[1].title().to_string(), EXTERNAL_LABEL);
            assert_eq!(items[1].value(), &labels[1]);
        }
    }

    #[::core::prelude::v1::test]
    fn transfer_creation_requires_an_eligible_account() {
        assert!(!can_create_transfer(&[], &[]));
        assert!(!can_create_transfer(
            &[card(1, PaymentCardType::Credit, Some(Currency::Rub))],
            &[]
        ));
        assert!(!can_create_transfer(
            &[card(2, PaymentCardType::Debit, None)],
            &[]
        ));
        assert!(!can_create_transfer(
            &[],
            &[account(
                3,
                Some(Currency::Rub),
                false,
                SavingsTerm::Unlimited
            )]
        ));
        assert!(!can_create_transfer(
            &[],
            &[account(4, None, true, SavingsTerm::Unlimited)]
        ));
        assert!(can_create_transfer(
            &[card(5, PaymentCardType::Debit, Some(Currency::Rub))],
            &[]
        ));
        assert!(can_create_transfer(
            &[],
            &[account(
                6,
                Some(Currency::Usd),
                true,
                SavingsTerm::Unlimited
            )]
        ));
    }

    #[::core::prelude::v1::test]
    fn new_form_defaults_and_edit_form_differ() {
        let cards = vec![
            card(1, PaymentCardType::Debit, Some(Currency::Rub)),
            card(2, PaymentCardType::Credit, Some(Currency::Rub)),
        ];
        let savings = vec![
            account(10, Some(Currency::Rub), true, SavingsTerm::Unlimited),
            account(11, Some(Currency::Rub), false, SavingsTerm::Unlimited),
        ];
        let fresh = TransferFormValues::new_for(&cards, &savings);
        assert_eq!(fresh.operation_type, Some(TransferOperationType::Transfer));
        assert_eq!(fresh.source, Some(TransferEndpoint::Card(1)));
        assert_eq!(fresh.destination, Some(TransferEndpoint::Savings(10)));
        assert_eq!(fresh.currency, Some(Currency::Rub));
        assert!(fresh.amount.is_empty());
        assert!(fresh.id.is_none());
        assert_eq!(
            fresh.date,
            Local::now().date_naive().format(DATE_FORMAT).to_string()
        );

        let record = TransferRecord {
            id: 42,
            operation_date: NaiveDate::from_ymd_opt(2026, 11, 21).unwrap(),
            operation_type: TransferOperationType::TopUp,
            source: TransferEndpoint::External,
            destination: TransferEndpoint::Card(1),
            amount: Decimal::from_str("12.50").unwrap(),
            currency: Currency::Rub,
            comment: Some("пополнение".to_owned()),
        };
        let editing = TransferFormValues::from_record(&record);
        assert_ne!(fresh, editing);
        assert_eq!(editing.id, Some(42));
        assert_eq!(editing.date, "21.11.2026");
        assert_eq!(editing.operation_type, Some(TransferOperationType::TopUp));
        assert_eq!(editing.source, Some(TransferEndpoint::External));
        assert_eq!(editing.destination, Some(TransferEndpoint::Card(1)));
        assert_eq!(editing.amount, "12.50");
        assert_eq!(editing.comment, "пополнение");
    }

    #[::core::prelude::v1::test]
    fn new_form_uses_top_up_when_only_one_account_is_available() {
        let cards = vec![card(1, PaymentCardType::Debit, Some(Currency::Rub))];
        let fresh = TransferFormValues::new_for(&cards, &[]);
        assert_eq!(fresh.operation_type, Some(TransferOperationType::TopUp));
        assert_eq!(fresh.source, Some(TransferEndpoint::External));
        assert_eq!(fresh.destination, Some(TransferEndpoint::Card(1)));
        let mut values = fresh.clone();
        values.amount = "10".to_owned();
        assert!(values.details().is_ok());
    }

    #[::core::prelude::v1::test]
    fn form_details_parse_amount_and_reject_invalid_values() {
        let mut values = TransferFormValues {
            date: "21.11.2026".to_owned(),
            operation_type: Some(TransferOperationType::Transfer),
            source: Some(TransferEndpoint::Card(1)),
            destination: Some(TransferEndpoint::Savings(10)),
            amount: "1 000,50".to_owned(),
            currency: Some(Currency::Rub),
            comment: "  тест  ".to_owned(),
            ..TransferFormValues::default()
        };
        assert!(values.details().is_err());

        values.amount = "1000,50".to_owned();
        let details = values.details().unwrap();
        assert_eq!(details.amount, Decimal::from_str("1000.50").unwrap());
        assert_eq!(details.comment.as_deref(), Some("тест"));

        values.destination = Some(TransferEndpoint::Card(1));
        assert!(values.details().is_err());

        values.comment = "x".repeat(MAX_TRANSFER_COMMENT_LENGTH + 1);
        assert!(values.details().is_err());
    }

    #[::core::prelude::v1::test]
    fn eligible_accounts_exclude_credit_and_inactive_and_honor_currency() {
        let cards = vec![
            card(1, PaymentCardType::Debit, Some(Currency::Rub)),
            card(2, PaymentCardType::Credit, Some(Currency::Rub)),
            card(3, PaymentCardType::Debit, Some(Currency::Usd)),
        ];
        let savings = vec![
            account(10, Some(Currency::Rub), true, SavingsTerm::Unlimited),
            account(11, Some(Currency::Rub), false, SavingsTerm::Unlimited),
            account(12, None, true, SavingsTerm::Unlimited),
        ];

        assert_eq!(eligible_cards(&cards, None).len(), 2);
        assert_eq!(eligible_cards(&cards, Some(Currency::Usd)).len(), 1);
        assert_eq!(eligible_savings(&savings, None).len(), 1);
        assert!(eligible_savings(&savings, Some(Currency::Usd)).is_empty());

        let (source, destination) = endpoint_options(
            TransferOperationType::TopUp,
            &cards,
            &savings,
            Some(Currency::Rub),
        );
        assert_eq!(
            source,
            vec![(TransferEndpoint::External, EXTERNAL_LABEL.to_owned())]
        );
        assert_eq!(destination.len(), 2);
        let (source, destination) = endpoint_options(
            TransferOperationType::Withdrawal,
            &cards,
            &savings,
            Some(Currency::Rub),
        );
        assert_eq!(
            destination,
            vec![(TransferEndpoint::Cash, CASH_LABEL.to_owned())]
        );
        assert_eq!(source.len(), 2);
    }

    #[::core::prelude::v1::test]
    fn balance_summary_keeps_currencies_separate_and_excludes_credit_limits() {
        let cards = vec![
            card(1, PaymentCardType::Debit, Some(Currency::Rub)),
            card(2, PaymentCardType::Credit, Some(Currency::Rub)),
            card(3, PaymentCardType::Debit, Some(Currency::Usd)),
        ];
        let savings = vec![
            account(10, Some(Currency::Rub), true, SavingsTerm::Unlimited),
            account(11, Some(Currency::Usd), false, SavingsTerm::Unlimited),
        ];
        let totals = balance_summary(&cards, &savings).unwrap();
        assert_eq!(
            totals[0],
            (
                Decimal::from_str("100").unwrap(),
                Decimal::from_str("100").unwrap()
            )
        );
        assert_eq!(
            totals[1],
            (
                Decimal::from_str("300").unwrap(),
                Decimal::from_str("110").unwrap()
            )
        );
    }

    #[::core::prelude::v1::test]
    fn transfer_column_widths_are_validated() {
        assert!(parse_column_widths(&format_column_widths(&DEFAULT_COLUMN_WIDTHS)).is_some());
        assert!(parse_column_widths("10,20,30,40,50,60,70,80").is_none());
        assert!(parse_column_widths(&format!("{},NaN", DEFAULT_COLUMN_WIDTHS[0])).is_none());
    }
}
