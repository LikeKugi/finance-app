//! Страница «Платёжные карты» на `gpui-kit`: CRUD дебетовых и кредитных карт,
//! фильтры, связи с накоплениями того же банка и постраничный вывод.
//! Транзакционность связей и проверки банка остаются в `finance-db`.

use std::rc::Rc;

use chrono::NaiveDate;
use finance_core::{
    canonical_amount, parse_amount, parse_non_negative_decimal, Bank, Currency, PaymentCardType,
    PaymentDocument, PaymentDocumentDetails, PaymentSystem, SavingsAccount,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const DATE_FORMAT: &str = "%Y-%m-%d";
const ROW_HEIGHT: f32 = 40.0;
const ALL_LABEL: &str = "Все";
const ALL_BANKS_LABEL: &str = "Все банки";
const NO_CURRENCY_LABEL: &str = "валюта не указана";
const RATE_WARNING: &str =
    "Условие сохраняется как возможность; приложение не подтверждает выполнение порога трат.";
/// Фиксированные ширины колонок 0..=8; колонка «Связанные накопления» занимает остаток,
/// «Действия» закреплены справа.
const FIXED_COLUMN_WIDTHS: [f32; 9] = [130.0, 95.0, 90.0, 100.0, 75.0, 105.0, 80.0, 125.0, 100.0];
const SAVINGS_COLUMN: usize = 9;
const ACTIONS_COLUMN_WIDTH: f32 = 180.0;
const HEADERS: [&str; 11] = [
    "Банк",
    "Система",
    "Тип",
    "Сумма",
    "Валюта",
    "Дата платежа",
    "Льгота, дни",
    "Условие ставки",
    "Минимум трат",
    "Связанные накопления",
    "Действия",
];

type Choice = Entity<SelectState<Vec<SharedString>>>;

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
    bank_id: Option<i64>,
    payment_system: Option<PaymentSystem>,
    card_type: Option<PaymentCardType>,
    amount: String,
    currency: Option<Currency>,
    due_date: String,
    grace_period_days: String,
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
    let (due_date, grace_period_days) = match card_type {
        PaymentCardType::Debit => (None, None),
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
            (Some(date), Some(days))
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
    Ok(PaymentDocumentDetails {
        bank_id,
        payment_system,
        card_type,
        amount,
        currency: input.currency,
        due_date,
        grace_period_days,
        affects_deposit_rate: input.affects_deposit_rate,
        min_monthly_spend,
        savings_ids,
    })
}

struct CardForm {
    id: Option<i64>,
    bank_id: Option<i64>,
    payment_system: Option<PaymentSystem>,
    card_type: Option<PaymentCardType>,
    currency: Option<Currency>,
    affects_deposit_rate: bool,
    savings_ids: Vec<i64>,
    available_savings: Vec<SavingsAccount>,
    amount: Entity<InputState>,
    due_date: Entity<InputState>,
    grace_period_days: Entity<InputState>,
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
            bank_id: self.bank_id,
            payment_system: self.payment_system,
            card_type: self.card_type,
            amount: self.amount.read(cx).value().to_string(),
            currency: self.currency,
            due_date: self.due_date.read(cx).value().to_string(),
            grace_period_days: self.grace_period_days.read(cx).value().to_string(),
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

        let mut page = Self {
            database,
            banks: Vec::new(),
            entries: Vec::new(),
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

        let mut form = CardForm {
            id: existing.map(|d| d.id),
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
            due_date: text_input(
                "2026-12-01",
                &existing
                    .and_then(|d| d.due_date)
                    .map_or_else(String::new, |d| d.format(DATE_FORMAT).to_string()),
                window,
                cx,
            ),
            grace_period_days: text_input(
                "Дни",
                &existing
                    .and_then(|d| d.grace_period_days)
                    .map_or_else(String::new, |d| d.to_string()),
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
                bank_id,
                payment_system,
                card_type,
                amount: String::new(),
                currency,
                due_date: String::new(),
                grace_period_days: String::new(),
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
                .w(px(820.0))
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

        let mut fields = div().flex().flex_col().gap_3().child(
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
                .child(field(
                    "Дата платежа (ГГГГ-ММ-ДД)",
                    Input::new(&form.due_date).into_any_element(),
                ))
                .child(field(
                    "Льготный период, дней",
                    Input::new(&form.grace_period_days).into_any_element(),
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

    fn render_table(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let stripe = cx.theme().muted;
        let count = self.entries.len();
        let header = div()
            .flex()
            .w_full()
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_1()
            .border_color(border)
            .font_weight(FontWeight::BOLD)
            .children(
                HEADERS
                    .into_iter()
                    .enumerate()
                    .map(|(index, title)| table_cell(index, title.to_owned(), border)),
            );
        let body = if count == 0 {
            div()
                .w_full()
                .p_4()
                .child("Платёжных карт по выбранным условиям нет.")
                .into_any_element()
        } else {
            uniform_list(
                "cards-rows",
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
        let document = self.entries[index].clone();
        let border = cx.theme().border;
        let id = document.id;
        let values = [
            document.bank_name.clone(),
            document.payment_system.code().to_owned(),
            document.card_type.title().to_owned(),
            canonical_amount(document.amount),
            document
                .currency
                .map_or(NO_CURRENCY_LABEL, Currency::code)
                .to_owned(),
            document
                .due_date
                .map_or_else(String::new, |d| d.format(DATE_FORMAT).to_string()),
            document
                .grace_period_days
                .map_or_else(String::new, |d| d.to_string()),
            if document.affects_deposit_rate {
                "Условно повышает"
            } else {
                "Нет"
            }
            .to_owned(),
            document
                .min_monthly_spend
                .map_or_else(String::new, canonical_amount),
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
        ];
        let edit_target = document.clone();
        let delete_target = document;
        div()
            .id(("card-row", id as usize))
            .flex()
            .w_full()
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_l_1()
            .border_r_1()
            .border_b_1()
            .border_color(border)
            .when(index % 2 == 1, |row| row.bg(stripe))
            .children(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(column, value)| table_cell(column, value, border)),
            )
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
                        Button::new(("card-edit", id as usize))
                            .xsmall()
                            .label("Изменить")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.open_form(Some(&edit_target), window, cx)
                            })),
                    )
                    .child(
                        Button::new(("card-delete", id as usize))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.confirm_delete(&delete_target, window, cx)
                            })),
                    ),
            )
    }
}

fn cell(width: f32, text: String, border: Hsla, separator: bool) -> Div {
    div()
        .w(px(width))
        .h_full()
        .flex()
        .items_center()
        .when(separator, |cell| cell.border_l_1().border_color(border))
        .px_2()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text)
}

fn table_cell(index: usize, text: String, border: Hsla) -> Div {
    match index {
        0..=8 => cell(FIXED_COLUMN_WIDTHS[index], text, border, index > 0),
        SAVINGS_COLUMN => div()
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
    use super::{
        build_details, currency_from_row, currency_row, CardInput, Currency, Database,
        PaymentCardType, PaymentDocument, PaymentSystem,
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
            bank_id: Some(1),
            payment_system: Some(PaymentSystem::Visa),
            card_type: Some(PaymentCardType::Debit),
            amount: String::new(),
            currency: None,
            due_date: String::new(),
            grace_period_days: String::new(),
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
        input.amount = "0".to_owned();
        save(&database, None, &input).unwrap();
        let entries = list(&database, None);
        assert_eq!(entries[0].currency, None);
        assert_eq!(entries[0].amount.to_string(), "0");
    }

    #[test]
    fn editing_card_preserves_changes_and_clears_currency() {
        let database = database_with_bank();
        let mut input = debit_input();
        input.amount = "10".to_owned();
        input.currency = Some(Currency::Rub);
        save(&database, None, &input).unwrap();
        let id = list(&database, None)[0].id;

        input.currency = Some(Currency::Usd);
        save(&database, Some(id), &input).unwrap();
        assert_eq!(list(&database, None)[0].currency, Some(Currency::Usd));
        input.currency = None;
        save(&database, Some(id), &input).unwrap();
        assert_eq!(list(&database, None)[0].currency, None);
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
