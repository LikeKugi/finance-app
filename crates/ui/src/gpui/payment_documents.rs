//! Страница «Платёжные карты» на `gpui-kit`: CRUD дебетовых и кредитных карт,
//! фильтры, связи с накоплениями того же банка и постраничный вывод.
//! Транзакционность связей и проверки банка остаются в `finance-db`.

use std::cmp::Ordering;
use std::rc::Rc;

use chrono::NaiveDate;
use finance_core::{
    canonical_amount, parse_amount, parse_non_negative_decimal, Bank, Currency, PaymentCardType,
    PaymentDocument, PaymentDocumentDetails, PaymentSystem, SavingsAccount,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::table::{
    Column, ColumnFixed, ColumnGroup, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const DATE_FORMAT: &str = "%Y-%m-%d";
const ALL_LABEL: &str = "Все";
const ALL_BANKS_LABEL: &str = "Все банки";
const NO_CURRENCY_LABEL: &str = "валюта не указана";
const RATE_WARNING: &str =
    "Условие сохраняется как возможность; приложение не подтверждает выполнение порога трат.";
const DEFAULT_COLUMN_WIDTHS: [f32; 13] = [
    140.0, 120.0, 90.0, 110.0, 100.0, 120.0, 100.0, 110.0, 130.0, 140.0, 110.0, 220.0, 160.0,
];
const ACTIONS_COLUMN_WIDTH: f32 = 160.0;
const MIN_COLUMN_WIDTH: f32 = 60.0;
const MAX_COLUMN_WIDTH: f32 = 1200.0;
const CARDS_COLUMN_WIDTHS_KEY: &str = "table_columns.payment_documents.v2";
const LEGACY_CARDS_COLUMN_WIDTHS_KEY: &str = "table_columns.payment_documents";
const COL_NAME: usize = 0;
const COL_BANK: usize = 1;
const COL_SYSTEM: usize = 2;
const COL_AMOUNT: usize = 4;
const COL_GRACE_PERIOD: usize = 6;
const COL_NEXT_PAYMENT_DATE: usize = 7;
const COL_NEXT_PAYMENT: usize = 8;
const COL_SAVINGS: usize = 11;
const COL_ACTIONS: usize = 12;
const HEADERS: [&str; 13] = [
    "Название",
    "Банк",
    "Система",
    "Тип",
    "Сумма",
    "Валюта",
    "Льгота, дни",
    "Дата",
    "Размер",
    "Условие ставки",
    "Минимум трат",
    "Связанные накопления",
    "Действия",
];
const NEXT_PAYMENT_GROUP_LABEL: &str = "Следующий платёж";
const NEXT_PAYMENT_GROUP_SPAN: usize = 2;

type Choice = Entity<SelectState<Vec<SharedString>>>;

/// Группа «Следующий платёж» объединяет колонки «Дата» и «Размер».
/// Пустые группы нужны, чтобы сохранить позиции всех остальных колонок,
/// включая закреплённую колонку «Действия».
fn next_payment_group_headers() -> Vec<Vec<ColumnGroup>> {
    vec![vec![
        ColumnGroup::new("", COL_NEXT_PAYMENT_DATE),
        ColumnGroup::new(NEXT_PAYMENT_GROUP_LABEL, NEXT_PAYMENT_GROUP_SPAN),
        ColumnGroup::new("", COL_SAVINGS - COL_NEXT_PAYMENT),
        ColumnGroup::new("", COL_ACTIONS - COL_SAVINGS),
    ]]
}

fn choice(
    items: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<PaymentDocumentsPage>,
) -> Choice {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn text_input(
    placeholder: &str,
    value: &str,
    window: &mut Window,
    cx: &mut Context<PaymentDocumentsPage>,
) -> Entity<InputState> {
    let placeholder = placeholder.to_owned();
    let value = value.to_owned();
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder);
        if !value.is_empty() {
            state.set_value(value, window, cx);
        }
        state
    })
}

fn system_labels() -> Vec<String> {
    PaymentSystem::ALL
        .iter()
        .map(|s| s.code().to_owned())
        .collect()
}

fn type_labels() -> Vec<String> {
    PaymentCardType::ALL
        .iter()
        .map(|t| t.title().to_owned())
        .collect()
}

/// Метки валют; первым идёт `first` («Все» для фильтра, «валюта не указана» для формы).
fn currency_labels(first: &str) -> Vec<String> {
    std::iter::once(first.to_owned())
        .chain(Currency::ALL.iter().map(|c| c.code().to_owned()))
        .collect()
}

fn currency_from_row(row: usize) -> Option<Currency> {
    row.checked_sub(1)
        .and_then(|i| Currency::ALL.get(i).copied())
}

fn currency_row(currency: Option<Currency>) -> usize {
    currency
        .and_then(|c| Currency::ALL.iter().position(|x| *x == c))
        .map_or(0, |i| i + 1)
}

fn savings_label(savings: &SavingsAccount) -> String {
    if savings.is_active {
        savings.title.clone()
    } else {
        format!("{} (неактивно)", savings.title)
    }
}

/// Строковые значения формы, не зависящие от GUI.
#[derive(Debug, Clone, PartialEq)]
struct CardInput {
    name: String,
    bank_id: Option<i64>,
    payment_system: Option<PaymentSystem>,
    card_type: Option<PaymentCardType>,
    amount: String,
    currency: Option<Currency>,
    due_date: String,
    grace_period_days: String,
    next_payment_amount: String,
    affects_deposit_rate: bool,
    min_monthly_spend: String,
    savings_ids: Vec<i64>,
}

/// Разбор и валидация формы; ошибки возвращаются пользователю, а не подавляются.
fn build_details(input: &CardInput) -> Result<PaymentDocumentDetails, String> {
    let bank_id = input.bank_id.ok_or_else(|| "Выберите банк".to_owned())?;
    let payment_system = input
        .payment_system
        .ok_or_else(|| "Выберите платёжную систему".to_owned())?;
    let card_type = input
        .card_type
        .ok_or_else(|| "Выберите тип карты".to_owned())?;
    let amount = match card_type {
        PaymentCardType::Debit => parse_non_negative_decimal(&input.amount),
        PaymentCardType::Credit => parse_amount(&input.amount),
    }
    .map_err(|error| error.to_string())?;
    let (due_date, grace_period_days, next_payment_amount) = match card_type {
        PaymentCardType::Debit => (None, None, None),
        PaymentCardType::Credit => {
            let date = NaiveDate::parse_from_str(input.due_date.trim(), DATE_FORMAT)
                .map_err(|_| "Введите дату платежа в формате ГГГГ-ММ-ДД".to_owned())?;
            let days = input
                .grace_period_days
                .trim()
                .parse::<i64>()
                .map_err(|_| "Введите целое неотрицательное число дней".to_owned())?;
            if days < 0 {
                return Err("Льготный период не может быть отрицательным".to_owned());
            }
            let next_payment = if input.next_payment_amount.trim().is_empty() {
                None
            } else {
                Some(
                    parse_non_negative_decimal(&input.next_payment_amount).map_err(|_| {
                        "Размер следующего платежа должен быть неотрицательным числом".to_owned()
                    })?,
                )
            };
            (Some(date), Some(days), next_payment)
        }
    };
    let (min_monthly_spend, savings_ids) = if input.affects_deposit_rate {
        (
            Some(parse_amount(&input.min_monthly_spend).map_err(|error| error.to_string())?),
            input.savings_ids.clone(),
        )
    } else {
        (None, Vec::new())
    };
    let name = input.name.trim();
    Ok(PaymentDocumentDetails {
        name: (!name.is_empty()).then(|| name.to_owned()),
        bank_id,
        payment_system,
        card_type,
        amount,
        currency: input.currency,
        due_date,
        grace_period_days,
        next_payment_amount,
        affects_deposit_rate: input.affects_deposit_rate,
        min_monthly_spend,
        savings_ids,
    })
}

struct CardForm {
    id: Option<i64>,
    name: Entity<InputState>,
    bank_id: Option<i64>,
    payment_system: Option<PaymentSystem>,
    card_type: Option<PaymentCardType>,
    currency: Option<Currency>,
    affects_deposit_rate: bool,
    savings_ids: Vec<i64>,
    available_savings: Vec<SavingsAccount>,
    amount: Entity<InputState>,
    due_date: Entity<DatePickerState>,
    grace_period_days: Entity<InputState>,
    next_payment_amount: Entity<InputState>,
    min_monthly_spend: Entity<InputState>,
    bank_select: Choice,
    system_select: Choice,
    type_select: Choice,
    currency_select: Choice,
    error: Option<String>,
    initial: CardInput,
    _subscriptions: Vec<Subscription>,
}

impl CardForm {
    fn input(&self, cx: &App) -> CardInput {
        CardInput {
            name: self.name.read(cx).value().to_string(),
            bank_id: self.bank_id,
            payment_system: self.payment_system,
            card_type: self.card_type,
            amount: self.amount.read(cx).value().to_string(),
            currency: self.currency,
            due_date: match self.due_date.read(cx).date() {
                Date::Single(Some(date)) => date.format(DATE_FORMAT).to_string(),
                _ => String::new(),
            },
            grace_period_days: self.grace_period_days.read(cx).value().to_string(),
            next_payment_amount: self.next_payment_amount.read(cx).value().to_string(),
            affects_deposit_rate: self.affects_deposit_rate,
            min_monthly_spend: self.min_monthly_spend.read(cx).value().to_string(),
            savings_ids: self.savings_ids.clone(),
        }
    }
}

pub(crate) struct PaymentDocumentsPage {
    database: Rc<Database>,
    banks: Vec<Bank>,
    entries: Vec<PaymentDocument>,
    table: Entity<TableState<CardsTable>>,
    bank_filter: Option<i64>,
    currency_filter: Option<Currency>,
    card_type_filter: Option<PaymentCardType>,
    payment_system_filter: Option<PaymentSystem>,
    bank_filter_select: Choice,
    system_filter_select: Choice,
    type_filter_select: Choice,
    currency_filter_select: Choice,
    message: Option<(String, bool)>,
    form: Option<CardForm>,
    confirm_discard: bool,
    _subscriptions: Vec<Subscription>,
}

fn selected_row(state: &Choice, cx: &App) -> Option<usize> {
    state.read(cx).selected_index(cx).map(|ix| ix.row)
}

impl PaymentDocumentsPage {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let bank_filter_select = choice(vec![ALL_BANKS_LABEL.to_owned()], 0, window, cx);
        let system_filter_select = choice(
            std::iter::once(ALL_LABEL.to_owned())
                .chain(system_labels())
                .collect(),
            0,
            window,
            cx,
        );
        let type_filter_select = choice(
            std::iter::once(ALL_LABEL.to_owned())
                .chain(type_labels())
                .collect(),
            0,
            window,
            cx,
        );
        let currency_filter_select = choice(currency_labels(ALL_LABEL), 0, window, cx);

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &bank_filter_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                let row = selected_row(state, cx);
                this.bank_filter = row
                    .and_then(|r| r.checked_sub(1))
                    .and_then(|i| this.banks.get(i))
                    .map(|bank| bank.id);
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &system_filter_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                this.payment_system_filter = selected_row(state, cx)
                    .and_then(|r| r.checked_sub(1))
                    .and_then(|i| PaymentSystem::ALL.get(i).copied());
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &type_filter_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                this.card_type_filter = selected_row(state, cx)
                    .and_then(|r| r.checked_sub(1))
                    .and_then(|i| PaymentCardType::ALL.get(i).copied());
                this.refresh(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &currency_filter_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                this.currency_filter = selected_row(state, cx).and_then(currency_from_row);
                this.refresh(cx);
            },
        ));

        let widths = load_column_widths(&database);
        let table_delegate = CardsTable::new(cx.entity().downgrade(), widths);
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
            bank_filter: None,
            currency_filter: None,
            card_type_filter: None,
            payment_system_filter: None,
            bank_filter_select,
            system_filter_select,
            type_filter_select,
            currency_filter_select,
            message: None,
            form: None,
            confirm_discard: false,
            _subscriptions: subscriptions,
        };
        page.reload(window, cx);
        page
    }

    /// Перечитывает банки и список карт; открытая форма и фильтры сохраняются.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let items: Vec<SharedString> = std::iter::once(ALL_BANKS_LABEL.to_owned())
            .chain(self.banks.iter().map(|bank| bank.name.clone()))
            .map(SharedString::from)
            .collect();
        let selected = self
            .bank_filter
            .and_then(|id| self.banks.iter().position(|bank| bank.id == id))
            .map_or(0, |i| i + 1);
        self.bank_filter_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(Some(IndexPath::default().row(selected)), window, cx);
        });
        self.refresh(cx);
    }

    fn save_column_widths(&mut self, widths: &[Pixels], cx: &mut Context<Self>) {
        let widths = column_widths_with_fixed_actions(
            widths.iter().map(|width| f32::from(*width)).collect(),
        );
        self.table
            .update(cx, |table, _| table.delegate_mut().set_widths(&widths));
        if let Err(error) = finance_db::set_app_setting(
            &self.database,
            CARDS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&widths),
        ) {
            self.error(format!("Не удалось сохранить ширину колонок: {error}"), cx);
        }
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some((message.into(), true));
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        match finance_db::list_payment_documents_filtered(
            &self.database,
            self.bank_filter,
            self.card_type_filter,
            self.payment_system_filter,
            self.currency_filter,
        ) {
            Ok(entries) => {
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_rows(entries.clone());
                    cx.notify();
                });
                self.entries = entries;
                if matches!(&self.message, Some((_, true))) {
                    self.message = None;
                }
                cx.notify();
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    fn has_active_filters(&self) -> bool {
        self.bank_filter.is_some()
            || self.currency_filter.is_some()
            || self.card_type_filter.is_some()
            || self.payment_system_filter.is_some()
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bank_filter = None;
        self.currency_filter = None;
        self.card_type_filter = None;
        self.payment_system_filter = None;
        for select in [
            &self.bank_filter_select,
            &self.system_filter_select,
            &self.type_filter_select,
            &self.currency_filter_select,
        ] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
            });
        }
        self.refresh(cx);
    }

    // ---- форма ----

    fn load_savings(
        &self,
        bank_id: Option<i64>,
    ) -> Result<Vec<SavingsAccount>, finance_db::DbError> {
        match bank_id {
            Some(bank_id) => finance_db::list_savings(&self.database, Some(bank_id), None, ""),
            None => Ok(Vec::new()),
        }
    }

    fn open_form(
        &mut self,
        existing: Option<&PaymentDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.message = None;
        self.confirm_discard = false;
        let bank_id = match existing {
            Some(document) => Some(document.bank_id),
            None => self.bank_filter,
        };
        let available_savings = match self.load_savings(bank_id) {
            Ok(list) => list,
            Err(error) => {
                self.error(error.to_string(), cx);
                return;
            }
        };
        let payment_system = Some(existing.map_or(PaymentSystem::Visa, |d| d.payment_system));
        let card_type = Some(existing.map_or(PaymentCardType::Debit, |d| d.card_type));
        let currency = existing.and_then(|d| d.currency);

        let bank_items: Vec<String> = self.banks.iter().map(|b| b.name.clone()).collect();
        let bank_row = bank_id
            .and_then(|id| self.banks.iter().position(|b| b.id == id))
            .unwrap_or(0);
        let bank_select = choice(bank_items, bank_row, window, cx);
        if bank_id.is_none() {
            bank_select.update(cx, |state, cx| state.set_selected_index(None, window, cx));
        }
        let system_row = PaymentSystem::ALL
            .iter()
            .position(|s| Some(*s) == payment_system)
            .unwrap_or(0);
        let system_select = choice(system_labels(), system_row, window, cx);
        let type_row = PaymentCardType::ALL
            .iter()
            .position(|t| Some(*t) == card_type)
            .unwrap_or(0);
        let type_select = choice(type_labels(), type_row, window, cx);
        let currency_select = choice(
            currency_labels(NO_CURRENCY_LABEL),
            currency_row(currency),
            window,
            cx,
        );

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &bank_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                let Some(row) = selected_row(state, cx) else {
                    return;
                };
                let Some(bank_id) = this.banks.get(row).map(|b| b.id) else {
                    return;
                };
                if this.form.as_ref().and_then(|f| f.bank_id) == Some(bank_id) {
                    return;
                }
                // Связи относятся к прежнему банку, поэтому сбрасываются.
                match this.load_savings(Some(bank_id)) {
                    Ok(list) => {
                        if let Some(form) = this.form.as_mut() {
                            form.bank_id = Some(bank_id);
                            form.savings_ids.clear();
                            form.available_savings = list;
                        }
                        cx.notify();
                    }
                    Err(error) => this.error(error.to_string(), cx),
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &system_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                if let (Some(form), Some(row)) = (this.form.as_mut(), selected_row(state, cx)) {
                    form.payment_system = PaymentSystem::ALL.get(row).copied();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &type_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                if let (Some(form), Some(row)) = (this.form.as_mut(), selected_row(state, cx)) {
                    form.card_type = PaymentCardType::ALL.get(row).copied();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &currency_select,
            window,
            |this: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                if let (Some(form), Some(row)) = (this.form.as_mut(), selected_row(state, cx)) {
                    form.currency = currency_from_row(row);
                }
                cx.notify();
            },
        ));

        let due_date = {
            let date = existing
                .and_then(|d| d.due_date)
                .or_else(|| NaiveDate::parse_from_str("2026-12-01", DATE_FORMAT).ok());
            cx.new(|cx| {
                let mut state = DatePickerState::new(window, cx).date_format(DATE_FORMAT);
                state.set_date(Date::Single(date), window, cx);
                state
            })
        };
        subscriptions.push(cx.subscribe_in(
            &due_date,
            window,
            |_, _, _: &DatePickerEvent, _, cx| cx.notify(),
        ));

        let mut form = CardForm {
            id: existing.map(|d| d.id),
            name: text_input(
                "Например, Основная карта",
                existing.and_then(|d| d.name.as_deref()).unwrap_or_default(),
                window,
                cx,
            ),
            bank_id,
            payment_system,
            card_type,
            currency,
            affects_deposit_rate: existing.is_some_and(|d| d.affects_deposit_rate),
            savings_ids: existing
                .map(|d| d.linked_savings.iter().map(|s| s.id).collect())
                .unwrap_or_default(),
            available_savings,
            amount: text_input(
                "Например, 100000",
                &existing.map_or_else(String::new, |d| canonical_amount(d.amount)),
                window,
                cx,
            ),
            due_date,
            grace_period_days: text_input(
                "Дни",
                &existing
                    .and_then(|d| d.grace_period_days)
                    .map_or_else(String::new, |d| d.to_string()),
                window,
                cx,
            ),
            next_payment_amount: text_input(
                "Например, 5000",
                &existing
                    .and_then(|d| d.next_payment_amount)
                    .map_or_else(String::new, canonical_amount),
                window,
                cx,
            ),
            min_monthly_spend: text_input(
                "Например, 10000",
                &existing
                    .and_then(|d| d.min_monthly_spend)
                    .map_or_else(String::new, canonical_amount),
                window,
                cx,
            ),
            bank_select,
            system_select,
            type_select,
            currency_select,
            error: None,
            initial: CardInput {
                name: String::new(),
                bank_id,
                payment_system,
                card_type,
                amount: String::new(),
                currency,
                due_date: String::new(),
                grace_period_days: String::new(),
                next_payment_amount: String::new(),
                affects_deposit_rate: false,
                min_monthly_spend: String::new(),
                savings_ids: Vec::new(),
            },
            _subscriptions: subscriptions,
        };
        form.initial = form.input(cx);
        self.form = Some(form);
        cx.notify();
        self.open_form_dialog(window, cx);
    }

    fn open_form_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        window.open_dialog(cx, move |dialog: Dialog, _, cx| {
            let (title, body) = this.update(cx, |page, cx| {
                let title = match page.form.as_ref().map(|form| form.id.is_some()) {
                    Some(true) => "Изменить платёжную карту",
                    _ => "Новая платёжная карта",
                };
                (title, page.render_form_dialog(cx))
            });
            dialog
                .title(title)
                .centered()
                .w(px(1100.0))
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(body)
        });
    }

    fn render_form_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.confirm_discard {
            self.render_discard_confirmation(cx)
        } else {
            self.render_form(cx)
        }
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
                        Button::new("card-continue-editing")
                            .label("Вернуться к форме")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.confirm_discard = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("card-discard-changes")
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
        let has_changes = self
            .form
            .as_ref()
            .is_some_and(|form| form.input(cx) != form.initial);
        if has_changes {
            self.confirm_discard = true;
        } else {
            self.form = None;
            self.confirm_discard = false;
            window.close_dialog(cx);
        }
        cx.notify();
    }

    /// Сохраняет форму; `true` — карта сохранена и диалог можно закрывать.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = self.form.as_ref() else {
            return false;
        };
        let input = form.input(cx);
        let id = form.id;
        let result = build_details(&input).and_then(|details| {
            finance_db::save_payment_document(&self.database, id, &details)
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(_) => {
                self.form = None;
                self.confirm_discard = false;
                self.refresh(cx);
                if !matches!(&self.message, Some((_, true))) {
                    self.message = Some(("Платёжная карта сохранена".to_owned(), false));
                    cx.notify();
                }
                true
            }
            Err(message) => {
                if let Some(form) = self.form.as_mut() {
                    form.error = Some(message);
                }
                cx.notify();
                false
            }
        }
    }

    fn confirm_delete(
        &mut self,
        document: &PaymentDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity();
        let id = document.id;
        let description = format!(
            "Удалить платёжную карту «{}» ({} {}, {})? Её связи с накоплениями также будут удалены.",
            document.bank_name,
            document.payment_system.code(),
            document.card_type.title(),
            canonical_amount(document.amount)
        );
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
                        match finance_db::delete_payment_document(&page.database, id) {
                            Ok(()) => {
                                if page.form.as_ref().and_then(|f| f.id) == Some(id) {
                                    page.form = None;
                                }
                                page.refresh(cx);
                                if !matches!(&page.message, Some((_, true))) {
                                    page.message =
                                        Some(("Платёжная карта удалена".to_owned(), false));
                                    cx.notify();
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

    fn render_filters(&self, cx: &mut Context<Self>) -> Div {
        let active = self.has_active_filters();
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
                        "Банк",
                        240.0,
                        Select::new(&self.bank_filter_select)
                            .placeholder(ALL_BANKS_LABEL)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Платёжная система",
                        170.0,
                        Select::new(&self.system_filter_select)
                            .placeholder(ALL_LABEL)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Тип карты",
                        160.0,
                        Select::new(&self.type_filter_select)
                            .placeholder(ALL_LABEL)
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Валюта",
                        150.0,
                        Select::new(&self.currency_filter_select)
                            .placeholder(ALL_LABEL)
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
                            .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
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
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let is_credit = form.card_type == Some(PaymentCardType::Credit);
        let amount_caption = match form.card_type {
            Some(PaymentCardType::Debit) => "Текущий баланс",
            Some(PaymentCardType::Credit) => "Кредитный лимит",
            None => "Сумма карты",
        };
        let field = |label: &'static str, control: AnyElement| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_size(px(12.0)).child(label))
                .child(control)
        };

        let mut fields = div().flex().flex_col().gap_3().child(field(
            "Название карты",
            Input::new(&form.name).into_any_element(),
        ));
        fields = fields.child(
            div()
                .flex()
                .gap_4()
                .child(field(
                    "Банк",
                    Select::new(&form.bank_select)
                        .placeholder("Выберите банк")
                        .into_any_element(),
                ))
                .child(field(
                    "Платёжная система",
                    Select::new(&form.system_select)
                        .placeholder("Выберите")
                        .into_any_element(),
                ))
                .child(field(
                    "Тип карты",
                    Select::new(&form.type_select)
                        .placeholder("Выберите")
                        .into_any_element(),
                )),
        );
        let mut amounts = div()
            .flex()
            .gap_4()
            .child(field(
                amount_caption,
                Input::new(&form.amount).into_any_element(),
            ))
            .child(field(
                "Валюта карты",
                Select::new(&form.currency_select)
                    .placeholder(NO_CURRENCY_LABEL)
                    .into_any_element(),
            ));
        if is_credit {
            amounts = amounts
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_size(px(12.0)).child("Дата следующего платежа"))
                        .child(
                            DatePicker::new(&form.due_date)
                                .placeholder("Выбрать дату")
                                .cleanable(true)
                                .small(),
                        ),
                )
                .child(field(
                    "Льготный период, дней",
                    Input::new(&form.grace_period_days).into_any_element(),
                ))
                .child(field(
                    "Размер следующего платежа",
                    Input::new(&form.next_payment_amount).into_any_element(),
                ));
        }
        fields = fields.child(amounts).child(
            Checkbox::new("card-affects-rate")
                .label("Условия карты могут влиять на ставку по накоплениям")
                .checked(form.affects_deposit_rate)
                .on_click(cx.listener(|page, checked: &bool, _, cx| {
                    if let Some(form) = page.form.as_mut() {
                        form.affects_deposit_rate = *checked;
                        if !*checked {
                            form.savings_ids.clear();
                        }
                    }
                    cx.notify();
                })),
        );
        if form.affects_deposit_rate {
            fields = fields.child(
                div()
                    .flex()
                    .gap_4()
                    .child(field(
                        "Минимум трат в месяц",
                        Input::new(&form.min_monthly_spend).into_any_element(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .child("Связанные накопления этого же банка"),
                            )
                            .child(if form.available_savings.is_empty() {
                                div().text_xs().text_color(muted).child(
                                    "Нет накоплений этого банка. Добавьте накопление, чтобы задать условие.",
                                ).into_any_element()
                            } else {
                                let mut list =
                                    div().id("card-savings-options").flex().flex_col().gap_1();
                                for savings in &form.available_savings {
                                    let id = savings.id;
                                    list = list.child(
                                        Checkbox::new(("card-savings", id as usize))
                                            .label(savings_label(savings))
                                            .checked(form.savings_ids.contains(&id))
                                            .on_click(cx.listener(
                                                move |page, checked: &bool, _, cx| {
                                                    if let Some(form) = page.form.as_mut() {
                                                        if *checked {
                                                            if !form.savings_ids.contains(&id) {
                                                                form.savings_ids.push(id);
                                                            }
                                                        } else {
                                                            form.savings_ids.retain(|x| *x != id);
                                                        }
                                                    }
                                                    cx.notify();
                                                },
                                            )),
                                    );
                                }
                                list.max_h(px(80.0)).overflow_y_scroll().into_any_element()
                            }),
                    ),
            );
            fields = fields.child(div().text_xs().text_color(warning).child(RATE_WARNING));
        }

        let mut panel = div().flex().flex_col().gap_3().p_4().child(fields);
        if let Some(error) = &form.error {
            panel = panel.child(div().text_color(danger).child(error.clone()));
        }
        panel
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("card-save")
                            .primary()
                            .label("Сохранить")
                            .on_click(cx.listener(|page, _, window, cx| {
                                if page.save(cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    )
                    .child(Button::new("card-cancel").label("Отмена").on_click(
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
    (widths.len() == HEADERS.len()
        && widths.iter().all(|width| {
            width.is_finite() && (MIN_COLUMN_WIDTH..=MAX_COLUMN_WIDTH).contains(width)
        }))
    .then_some(widths)
}

fn column_widths_with_fixed_actions(mut widths: Vec<f32>) -> Vec<f32> {
    if let Some(width) = widths.get_mut(COL_ACTIONS) {
        *width = ACTIONS_COLUMN_WIDTH;
    }
    widths
}

fn parse_saved_column_widths(value: &str) -> Option<Vec<f32>> {
    parse_column_widths(value).map(column_widths_with_fixed_actions)
}

/// Ширины сохранялись по индексам; после перестановки колонок возвращаем каждой её ширину.
fn migrate_legacy_column_widths(mut widths: Vec<f32>) -> Vec<f32> {
    widths.swap(COL_GRACE_PERIOD, COL_NEXT_PAYMENT_DATE);
    widths
}

fn load_column_widths(database: &Database) -> Vec<f32> {
    match finance_db::get_app_setting(database, CARDS_COLUMN_WIDTHS_KEY) {
        Ok(Some(value)) => {
            return parse_saved_column_widths(&value).unwrap_or_else(|| {
                eprintln!(
                    "Некорректная сохранённая ширина колонок карт; использованы значения по умолчанию"
                );
                DEFAULT_COLUMN_WIDTHS.to_vec()
            });
        }
        Ok(None) => {}
        Err(error) => eprintln!("Не удалось прочитать ширину колонок карт: {error}"),
    }

    match finance_db::get_app_setting(database, LEGACY_CARDS_COLUMN_WIDTHS_KEY) {
        Ok(Some(value)) => parse_saved_column_widths(&value)
            .map(migrate_legacy_column_widths)
            .unwrap_or_else(|| {
                eprintln!(
                    "Некорректная сохранённая ширина колонок карт; использованы значения по умолчанию"
                );
                DEFAULT_COLUMN_WIDTHS.to_vec()
            }),
        Ok(None) => DEFAULT_COLUMN_WIDTHS.to_vec(),
        Err(error) => {
            eprintln!("Не удалось прочитать ширину колонок карт: {error}");
            DEFAULT_COLUMN_WIDTHS.to_vec()
        }
    }
}

fn card_name_key(document: &PaymentDocument) -> String {
    document.name.as_deref().unwrap_or_default().to_lowercase()
}

fn default_order(a: &PaymentDocument, b: &PaymentDocument) -> Ordering {
    a.bank_name
        .to_lowercase()
        .cmp(&b.bank_name.to_lowercase())
        .then_with(|| card_name_key(a).cmp(&card_name_key(b)))
        .then(a.id.cmp(&b.id))
}

fn sort_cards(rows: &mut [PaymentDocument], sort: Option<(usize, ColumnSort)>) {
    let Some((column, direction)) = sort else {
        rows.sort_by(default_order);
        return;
    };
    rows.sort_by(|a, b| {
        let ordering = match column {
            COL_NAME => card_name_key(a).cmp(&card_name_key(b)),
            COL_BANK => a.bank_name.to_lowercase().cmp(&b.bank_name.to_lowercase()),
            COL_SYSTEM => a
                .payment_system
                .code()
                .to_lowercase()
                .cmp(&b.payment_system.code().to_lowercase()),
            COL_AMOUNT => a.amount.cmp(&b.amount),
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

struct CardsTable {
    page: WeakEntity<PaymentDocumentsPage>,
    rows: Vec<PaymentDocument>,
    widths: Vec<f32>,
    sort: Option<(usize, ColumnSort)>,
}

impl CardsTable {
    fn new(page: WeakEntity<PaymentDocumentsPage>, widths: Vec<f32>) -> Self {
        Self {
            page,
            rows: Vec::new(),
            widths,
            sort: None,
        }
    }

    fn set_rows(&mut self, rows: Vec<PaymentDocument>) {
        self.rows = rows;
        sort_cards(&mut self.rows, self.sort);
    }

    fn set_widths(&mut self, widths: &[f32]) {
        if widths.len() == self.widths.len() {
            self.widths = column_widths_with_fixed_actions(widths.to_vec());
        }
    }
}

impl TableDelegate for CardsTable {
    fn columns_count(&self, _: &App) -> usize {
        HEADERS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn group_headers(&self, _: &App) -> Option<Vec<Vec<ColumnGroup>>> {
        Some(next_payment_group_headers())
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
            .child(HEADERS[col_ix])
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let is_actions = col_ix == COL_ACTIONS;
        let column = Column::new(HEADERS[col_ix], HEADERS[col_ix])
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
            COL_NAME | COL_BANK | COL_SYSTEM | COL_AMOUNT => column.sort(direction),
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
        sort_cards(&mut self.rows, self.sort);
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
            .child("Платёжных карт по выбранным условиям нет.")
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let document = &self.rows[row_ix];
        let text = |value: String| div().child(value).into_any_element();
        let content = match col_ix {
            COL_NAME => text(document.name.clone().unwrap_or_default()),
            COL_BANK => text(document.bank_name.clone()),
            COL_SYSTEM => text(document.payment_system.code().to_owned()),
            3 => text(document.card_type.title().to_owned()),
            COL_AMOUNT => div()
                .w_full()
                .text_right()
                .child(canonical_amount(document.amount))
                .into_any_element(),
            5 => text(
                document
                    .currency
                    .map_or(NO_CURRENCY_LABEL, Currency::code)
                    .to_owned(),
            ),
            COL_GRACE_PERIOD => text(
                document
                    .grace_period_days
                    .map_or_else(String::new, |d| d.to_string()),
            ),
            COL_NEXT_PAYMENT_DATE => text(
                document
                    .due_date
                    .map_or_else(String::new, |d| d.format(DATE_FORMAT).to_string()),
            ),
            COL_NEXT_PAYMENT => text(
                document
                    .next_payment_amount
                    .map_or_else(String::new, canonical_amount),
            ),
            9 => text(
                if document.affects_deposit_rate {
                    "Условно повышает"
                } else {
                    "Нет"
                }
                .to_owned(),
            ),
            10 => text(
                document
                    .min_monthly_spend
                    .map_or_else(String::new, canonical_amount),
            ),
            COL_SAVINGS => text(
                document
                    .linked_savings
                    .iter()
                    .map(|s| {
                        if s.is_active {
                            s.title.clone()
                        } else {
                            format!("{} (неактивно)", s.title)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            COL_ACTIONS => {
                let id = document.id;
                let edit_target = document.clone();
                let delete_target = document.clone();
                let edit_page = self.page.clone();
                let delete_page = self.page.clone();
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_full()
                    .h_full()
                    .gap_1()
                    .child(
                        Button::new(("card-edit", id as usize))
                            .xsmall()
                            .label("Изменить")
                            .on_click(move |_, window, cx| {
                                let _ = edit_page.update(cx, |page, cx| {
                                    page.open_form(Some(&edit_target), window, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(("card-delete", id as usize))
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
        div()
            .w_full()
            .h_full()
            .when(col_ix > 0, |cell| cell.border_l_1().border_color(border))
            .child(content)
            .into_any_element()
    }
}

impl Render for PaymentDocumentsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let success = cx.theme().success;
        let header = div()
            .flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(24.0))
                    .font_weight(FontWeight::BOLD)
                    .child("Платёжные карты"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("new-card")
                            .primary()
                            .large()
                            .label("Добавить карту")
                            .icon(IconName::Plus)
                            .disabled(self.form.is_some() || self.banks.is_empty())
                            .on_click(
                                cx.listener(|page, _, window, cx| page.open_form(None, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("refresh-cards")
                            .secondary()
                            .large()
                            .label("Обновить")
                            .icon(IconName::Replace)
                            .on_click(cx.listener(|page, _, window, cx| page.reload(window, cx))),
                    ),
            );
        let mut root = div()
            .id("payment-documents-page")
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(header);
        if self.banks.is_empty() {
            root = root.child(
                "Сначала добавьте банк в разделе «Справочники», чтобы создать платёжную карту.",
            );
        }
        root = root.child(self.render_filters(cx));
        if let Some((message, is_error)) = self.message.clone() {
            root = root.child(
                div()
                    .text_color(if is_error { danger } else { success })
                    .child(message),
            );
        }
        root.child(self.render_table(cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::table::ColumnSort;

    use super::{
        build_details, currency_from_row, currency_row, format_column_widths, load_column_widths,
        next_payment_group_headers, parse_column_widths, sort_cards, CardInput, Currency, Database,
        PaymentCardType, PaymentDocument, PaymentSystem, ACTIONS_COLUMN_WIDTH, COL_ACTIONS,
        COL_AMOUNT, COL_BANK, COL_GRACE_PERIOD, COL_NAME, COL_NEXT_PAYMENT, COL_NEXT_PAYMENT_DATE,
        COL_SYSTEM, DEFAULT_COLUMN_WIDTHS, HEADERS, LEGACY_CARDS_COLUMN_WIDTHS_KEY,
        NEXT_PAYMENT_GROUP_LABEL, NEXT_PAYMENT_GROUP_SPAN,
    };

    fn database_with_bank() -> Database {
        let database = Database::open_in_memory().unwrap();
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
            include_str!("../../../db/migrations/V13__payment_document_next_payment.sql"),
        ] {
            database.execute_batch(sql).unwrap();
        }
        database
            .execute_batch("INSERT INTO banks (id, name) VALUES (1, 'Банк Альфа');")
            .unwrap();
        database
    }

    fn debit_input() -> CardInput {
        CardInput {
            name: String::new(),
            bank_id: Some(1),
            payment_system: Some(PaymentSystem::Visa),
            card_type: Some(PaymentCardType::Debit),
            amount: String::new(),
            currency: None,
            due_date: String::new(),
            grace_period_days: String::new(),
            next_payment_amount: String::new(),
            affects_deposit_rate: false,
            min_monthly_spend: String::new(),
            savings_ids: Vec::new(),
        }
    }

    fn save(database: &Database, id: Option<i64>, input: &CardInput) -> Result<(), String> {
        let details = build_details(input)?;
        finance_db::save_payment_document(database, id, &details)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn list(database: &Database, currency: Option<Currency>) -> Vec<PaymentDocument> {
        finance_db::list_payment_documents_filtered(database, None, None, None, currency).unwrap()
    }

    #[test]
    fn currency_filter_excludes_unknown_currency_and_all_restores_everything() {
        let database = database_with_bank();
        database
            .execute_batch(
                "INSERT INTO payment_documents (
                     id, bank_id, payment_system, card_type, amount, currency
                 ) VALUES
                     (1, 1, 'VISA', 'debit', '10', 'RUB'),
                     (2, 1, 'MIR', 'debit', '20', 'USD'),
                     (3, 1, 'MasterCard', 'debit', '30', NULL);",
            )
            .unwrap();
        let usd = list(&database, Some(Currency::Usd));
        assert_eq!(usd.iter().map(|e| e.id).collect::<Vec<_>>(), [2]);
        let all = list(&database, None);
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|e| e.currency.is_none()));
    }

    #[test]
    fn new_debit_card_saves_zero_balance_without_automatic_currency() {
        let database = database_with_bank();
        let mut input = debit_input();
        input.name = "Основная".to_owned();
        input.amount = "0".to_owned();
        save(&database, None, &input).unwrap();
        let entries = list(&database, None);
        assert_eq!(entries[0].name.as_deref(), Some("Основная"));
        assert_eq!(entries[0].currency, None);
        assert_eq!(entries[0].amount.to_string(), "0");
    }

    #[test]
    fn editing_card_preserves_changes_and_clears_currency() {
        let database = database_with_bank();
        let mut input = debit_input();
        input.amount = "10".to_owned();
        input.name = "Основная".to_owned();
        input.currency = Some(Currency::Rub);
        save(&database, None, &input).unwrap();
        let id = list(&database, None)[0].id;

        input.currency = Some(Currency::Usd);
        input.name = "Для поездок".to_owned();
        save(&database, Some(id), &input).unwrap();
        assert_eq!(list(&database, None)[0].currency, Some(Currency::Usd));
        assert_eq!(
            list(&database, None)[0].name.as_deref(),
            Some("Для поездок")
        );
        input.currency = None;
        input.name.clear();
        save(&database, Some(id), &input).unwrap();
        assert_eq!(list(&database, None)[0].currency, None);
        assert_eq!(list(&database, None)[0].name, None);
    }

    #[test]
    fn negative_debit_balance_and_zero_credit_limit_are_rejected() {
        let database = database_with_bank();
        let mut input = debit_input();
        input.amount = "-1".to_owned();
        assert!(save(&database, None, &input).is_err());

        input.card_type = Some(PaymentCardType::Credit);
        input.amount = "0".to_owned();
        input.due_date = "2026-12-01".to_owned();
        input.grace_period_days = "55".to_owned();
        assert!(save(&database, None, &input).is_err());
        assert!(list(&database, None).is_empty());
    }

    #[test]
    fn credit_card_requires_valid_date_and_non_negative_grace_period() {
        let mut input = debit_input();
        input.card_type = Some(PaymentCardType::Credit);
        input.amount = "50000".to_owned();
        input.due_date = "не дата".to_owned();
        input.grace_period_days = "55".to_owned();
        assert!(build_details(&input).is_err());
        input.due_date = "2026-12-01".to_owned();
        input.grace_period_days = "-1".to_owned();
        assert!(build_details(&input).is_err());
        input.grace_period_days = "abc".to_owned();
        assert!(build_details(&input).is_err());
        input.grace_period_days = "55".to_owned();
        let details = build_details(&input).unwrap();
        assert_eq!(details.grace_period_days, Some(55));
        assert!(details.due_date.is_some());
    }

    #[test]
    fn debit_card_ignores_credit_fields_and_rate_off_drops_links() {
        let mut input = debit_input();
        input.amount = "100".to_owned();
        input.due_date = "мусор".to_owned();
        input.min_monthly_spend = "5".to_owned();
        input.savings_ids = vec![7];
        let details = build_details(&input).unwrap();
        assert_eq!(details.due_date, None);
        assert_eq!(details.grace_period_days, None);
        assert_eq!(details.min_monthly_spend, None);
        assert!(details.savings_ids.is_empty());
    }

    #[test]
    fn rate_condition_requires_valid_minimum_spend() {
        let mut input = debit_input();
        input.amount = "100".to_owned();
        input.affects_deposit_rate = true;
        assert!(build_details(&input).is_err());
        input.min_monthly_spend = "10000".to_owned();
        input.savings_ids = vec![3];
        let details = build_details(&input).unwrap();
        assert_eq!(details.savings_ids, [3]);
        assert!(details.min_monthly_spend.is_some());
    }

    fn card(
        id: i64,
        name: &str,
        bank: &str,
        system: PaymentSystem,
        amount: i64,
    ) -> PaymentDocument {
        PaymentDocument {
            id,
            name: Some(name.to_owned()),
            bank_id: 1,
            bank_name: bank.to_owned(),
            payment_system: system,
            card_type: PaymentCardType::Debit,
            amount: rust_decimal::Decimal::from(amount),
            currency: None,
            due_date: None,
            grace_period_days: None,
            next_payment_amount: None,
            affects_deposit_rate: false,
            min_monthly_spend: None,
            linked_savings: Vec::new(),
        }
    }

    fn sorted_ids(column: usize, direction: ColumnSort) -> Vec<i64> {
        let mut rows = vec![
            card(1, "Зарплатная", "Банк Бета", PaymentSystem::Mir, 20),
            card(2, "Альфа", "Банк Альфа", PaymentSystem::Visa, 100),
            card(3, "Гамма", "Банк Альфа", PaymentSystem::MasterCard, 3),
        ];
        sort_cards(&mut rows, Some((column, direction)));
        rows.into_iter().map(|card| card.id).collect()
    }

    #[test]
    fn card_sorting_covers_requested_columns_in_both_directions() {
        for (column, asc, desc) in [
            (COL_NAME, [2, 3, 1], [1, 3, 2]),
            (COL_BANK, [2, 3, 1], [1, 2, 3]),
            (COL_SYSTEM, [3, 1, 2], [2, 1, 3]),
            (COL_AMOUNT, [3, 1, 2], [2, 1, 3]),
        ] {
            assert_eq!(
                sorted_ids(column, ColumnSort::Ascending),
                asc,
                "asc {column}"
            );
            assert_eq!(
                sorted_ids(column, ColumnSort::Descending),
                desc,
                "desc {column}"
            );
        }
    }

    #[test]
    fn column_widths_round_trip_and_reject_invalid_values() {
        let text = format_column_widths(&DEFAULT_COLUMN_WIDTHS);
        assert_eq!(
            parse_column_widths(&text),
            Some(DEFAULT_COLUMN_WIDTHS.to_vec())
        );
        assert_eq!(parse_column_widths("100,200"), None);
        assert_eq!(parse_column_widths(&text.replacen("140", "5", 1)), None);
        assert_eq!(parse_column_widths(&text.replacen("140", "abc", 1)), None);
    }

    #[test]
    fn next_payment_header_groups_date_and_amount() {
        assert_eq!(HEADERS[COL_GRACE_PERIOD], "Льгота, дни");
        assert_eq!(HEADERS[COL_NEXT_PAYMENT_DATE], "Дата");
        assert_eq!(HEADERS[COL_NEXT_PAYMENT], "Размер");

        let rows = next_payment_group_headers();
        assert_eq!(rows.len(), 1);
        let groups = &rows[0];
        assert_eq!(
            groups.iter().map(|group| group.span).sum::<usize>(),
            HEADERS.len()
        );

        let grouped_from: usize = groups
            .iter()
            .take_while(|group| group.label.is_empty())
            .map(|group| group.span)
            .sum();
        assert_eq!(grouped_from, COL_NEXT_PAYMENT_DATE);
        assert_eq!(groups[1].label, NEXT_PAYMENT_GROUP_LABEL);
        assert_eq!(groups[1].span, NEXT_PAYMENT_GROUP_SPAN);
    }

    #[test]
    fn legacy_column_widths_follow_swapped_columns() {
        let database = database_with_bank();
        let mut legacy_widths = DEFAULT_COLUMN_WIDTHS;
        legacy_widths[COL_GRACE_PERIOD] = 175.0;
        legacy_widths[COL_NEXT_PAYMENT_DATE] = 195.0;
        finance_db::set_app_setting(
            &database,
            LEGACY_CARDS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&legacy_widths),
        )
        .unwrap();

        let widths = load_column_widths(&database);

        assert_eq!(widths[COL_GRACE_PERIOD], 195.0);
        assert_eq!(widths[COL_NEXT_PAYMENT_DATE], 175.0);
        assert_eq!(
            widths[COL_NEXT_PAYMENT],
            DEFAULT_COLUMN_WIDTHS[COL_NEXT_PAYMENT]
        );
    }

    #[test]
    fn saved_column_widths_keep_other_columns_and_fix_actions_width() {
        let database = database_with_bank();
        let mut saved_widths = DEFAULT_COLUMN_WIDTHS;
        saved_widths[COL_BANK] = 245.0;
        saved_widths[COL_ACTIONS] = 235.0;
        finance_db::set_app_setting(
            &database,
            super::CARDS_COLUMN_WIDTHS_KEY,
            &format_column_widths(&saved_widths),
        )
        .unwrap();

        let mut expected = saved_widths;
        expected[COL_ACTIONS] = 160.0;
        assert_eq!(load_column_widths(&database), expected);
    }

    #[test]
    fn missing_or_invalid_saved_column_widths_use_fixed_action_default() {
        let database = database_with_bank();
        let mut expected = DEFAULT_COLUMN_WIDTHS;
        expected[COL_ACTIONS] = ACTIONS_COLUMN_WIDTH;
        assert_eq!(load_column_widths(&database), expected);

        finance_db::set_app_setting(&database, super::CARDS_COLUMN_WIDTHS_KEY, "invalid").unwrap();
        assert_eq!(load_column_widths(&database), expected);
    }

    #[test]
    fn changing_column_widths_keeps_actions_fixed_and_other_widths() {
        let mut widths = DEFAULT_COLUMN_WIDTHS.to_vec();
        widths[COL_BANK] = 275.0;
        widths[COL_ACTIONS] = 230.0;

        let widths = super::column_widths_with_fixed_actions(widths);

        assert_eq!(widths[COL_BANK], 275.0);
        assert_eq!(widths[COL_ACTIONS], ACTIONS_COLUMN_WIDTH);
    }

    #[test]
    fn currency_rows_round_trip() {
        assert_eq!(currency_from_row(0), None);
        for currency in Currency::ALL {
            assert_eq!(
                currency_from_row(currency_row(Some(currency))),
                Some(currency)
            );
        }
        assert_eq!(currency_row(None), 0);
    }
}
