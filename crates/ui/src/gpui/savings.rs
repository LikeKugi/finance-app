//! Страница «Накопления» на `gpui-kit`. Поведение соответствует
//! `savings_page.rs`; SQL и финансовые правила остаются в `finance-db` и
//! `finance-core`.

use std::rc::Rc;

use chrono::NaiveDate;
use finance_core::{
    canonical_amount, parse_amount, parse_non_negative_decimal, Bank, Currency,
    InterestDestination, SavingsAccount, SavingsDetails, SavingsTerm,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const SAVINGS_PAGE_SIZE: usize = 100;
const DATE_FORMAT: &str = "%Y-%m-%d";
const ROW_HEIGHT: f32 = 40.0;
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
    "Ставка при условии",
    "Порог",
    "Срок",
    "Дата открытия",
    "Дата начисления процентов",
    "Назначение процентов",
    "Статус",
    "Действия",
];
const COLUMN_WIDTHS: [f32; 13] = [
    80.0, 100.0, 75.0, 75.0, 95.0, 105.0, 75.0, 80.0, 90.0, 105.0, 120.0, 75.0, 145.0,
];
const FOOTER_NOTE: &str = "Даты открытия и начисления процентов сохраняются как справочные значения; процентный доход здесь не рассчитывается без истории остатка и правил начисления банка. Повышенная ставка показана как условие, а не как подтверждённая доходность.";
const FOOTER_WARNING: &str =
    "Валюта накопления указывается отдельно; фактические начисления процентов здесь не рассчитываются.";

type Choice = Entity<SelectState<Vec<SharedString>>>;

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

fn format_date(date: NaiveDate) -> String {
    date.format("%d.%m.%Y").to_string()
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
    opened_date: Entity<InputState>,
    interest_accrual_date: Entity<InputState>,
    term_end_date: Entity<InputState>,
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
    current_page: usize,
    message: Option<(String, bool)>,
    search: Entity<InputState>,
    bank_filter: Option<i64>,
    bank_filter_select: Choice,
    currency_filter: Option<Currency>,
    currency_filter_select: Choice,
    active_filter: Option<bool>,
    active_filter_select: Choice,
    form: Option<Form>,
    _subscriptions: Vec<Subscription>,
}

impl SavingsPage {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Название или банк"));
        let bank_filter_select = choice(vec![ALL_BANKS_LABEL.to_owned()], Some(0), window, cx);
        let currency_filter_select = choice(currency_labels(ALL_LABEL), Some(0), window, cx);
        let active_filter_select = choice(status_labels(), Some(0), window, cx);

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
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.bank_filter = value.as_ref().and_then(|label| {
                        this.banks
                            .iter()
                            .find(|bank| bank.name == label.as_ref())
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
        ];
        subscriptions.shrink_to_fit();

        let mut page = Self {
            database,
            banks: Vec::new(),
            entries: Vec::new(),
            current_page: 0,
            message: None,
            search,
            bank_filter: None,
            bank_filter_select,
            currency_filter: None,
            currency_filter_select,
            active_filter: None,
            active_filter_select,
            form: None,
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
        let items: Vec<SharedString> = std::iter::once(ALL_BANKS_LABEL.to_owned())
            .chain(self.banks.iter().map(|bank| bank.name.clone()))
            .map(SharedString::from)
            .collect();
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
                self.entries = entries;
                self.current_page = 0;
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
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.bank_filter = None;
        self.currency_filter = None;
        self.active_filter = None;
        for select in [
            &self.bank_filter_select,
            &self.currency_filter_select,
            &self.active_filter_select,
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
        let opened_date = text_input(&initial.opened_date, "ГГГГ-ММ-ДД", window, cx);
        let interest_accrual_date =
            text_input(&initial.interest_accrual_date, "ГГГГ-ММ-ДД", window, cx);
        let term_end_date = text_input(&initial.term_end_date, "ГГГГ-ММ-ДД", window, cx);

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

        let subscriptions = vec![
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
        ];

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
        cx.notify();
    }

    /// Текущие значения формы, собранные из полей ввода.
    fn read_form(&self, cx: &App) -> Option<SavingsForm> {
        let form = self.form.as_ref()?;
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
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
            opened_date: text(&form.opened_date),
            interest_accrual_date: text(&form.interest_accrual_date),
            term_type: form.term_type,
            term_end_date: text(&form.term_end_date),
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

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(values) = self.read_form(cx) else {
            return;
        };
        let details = match values.details() {
            Ok(details) => details,
            Err(error) => {
                self.set_form_error(error, cx);
                return;
            }
        };
        match finance_db::save_savings(&self.database, values.id, &details) {
            Ok(_) => {
                self.form = None;
                if self.refresh(cx) {
                    self.message = Some(("Накопление сохранено".to_owned(), false));
                }
                cx.notify();
            }
            Err(error) => self.set_form_error(error.to_string(), cx),
        }
    }

    fn request_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = match (self.read_form(cx), self.form.as_ref()) {
            (Some(current), Some(form)) => current != form.snapshot,
            _ => false,
        };
        if !dirty {
            self.form = None;
            cx.notify();
            return;
        }
        let this = cx.entity();
        window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
            let this = this.clone();
            alert
                .title("Несохранённые изменения")
                .description("Отменить изменения и закрыть форму?")
                .ok_text("Отменить изменения")
                .cancel_text("Продолжить редактирование")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |page, cx| {
                        page.form = None;
                        cx.notify();
                    });
                    true
                })
        });
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

    fn render_filters(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.has_active_filters(cx);
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(220.0))
                    .child(Input::new(&self.search).cleanable(true)),
            )
            .child(
                div()
                    .w(px(180.0))
                    .child(Select::new(&self.bank_filter_select).placeholder("Банк")),
            )
            .child(
                div()
                    .w(px(110.0))
                    .child(Select::new(&self.currency_filter_select).placeholder("Валюта")),
            )
            .child(
                div()
                    .w(px(150.0))
                    .child(Select::new(&self.active_filter_select).placeholder("Статус")),
            )
            .child(
                Button::new("filters-apply")
                    .primary()
                    .label("Применить")
                    .on_click(cx.listener(|page, _, _, cx| page.apply_filters(cx))),
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
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let limited = form.term_type == Some(SavingsTerm::Limited);
        let labelled = |label: &'static str, width: f32, child: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .w(px(width))
                .child(div().text_color(muted).child(label))
                .child(child)
        };
        let section = |title: &'static str| div().font_weight(FontWeight::BOLD).child(title);

        let mut fields = div()
            .id("savings-form-fields")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .child(section("Основное"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap_3()
                    .child(labelled(
                        "Название",
                        280.0,
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
                    ))
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
            )
            .child(section("Сумма и ставки"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap_3()
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
                    ),
            );
        if form.use_threshold {
            fields = fields.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(labelled(
                        "Пороговая сумма трат",
                        280.0,
                        Input::new(&form.threshold_amount).into_any_element(),
                    ))
                    .child(labelled(
                        "Ставка при выполнении условий, %",
                        280.0,
                        Input::new(&form.rate_after_threshold).into_any_element(),
                    )),
            );
        }
        fields = fields.child(section("Даты и срок")).child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .child(labelled(
                    "Дата открытия (ГГГГ-ММ-ДД)",
                    220.0,
                    Input::new(&form.opened_date).into_any_element(),
                ))
                .child(labelled(
                    "Дата начисления процентов (ГГГГ-ММ-ДД)",
                    280.0,
                    Input::new(&form.interest_accrual_date).into_any_element(),
                ))
                .child(labelled(
                    "Срок",
                    200.0,
                    Select::new(&form.term_select)
                        .placeholder("Выберите срок")
                        .into_any_element(),
                ))
                .when(limited, |row| {
                    row.child(labelled(
                        "Дата закрытия (ГГГГ-ММ-ДД)",
                        220.0,
                        Input::new(&form.term_end_date).into_any_element(),
                    ))
                }),
        );
        fields = fields.child(labelled(
            "Выплата процентов",
            320.0,
            Select::new(&form.destination_select)
                .placeholder("Выберите назначение")
                .into_any_element(),
        ));

        Some(
            div()
                .flex_1()
                .min_h_0()
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
                            "Изменить накопление"
                        } else {
                            "Новое накопление"
                        }),
                )
                .child(fields)
                .when_some(form.error.clone(), |panel, error| {
                    panel.child(div().text_color(danger).child(error))
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("form-save")
                                .primary()
                                .label("Сохранить")
                                .on_click(cx.listener(|page, _, _, cx| page.save(cx))),
                        )
                        .child(Button::new("form-cancel").label("Отмена").on_click(
                            cx.listener(|page, _, window, cx| page.request_cancel(window, cx)),
                        )),
                ),
        )
    }

    fn page_count(&self) -> usize {
        self.entries.len().div_ceil(SAVINGS_PAGE_SIZE)
    }

    fn render_table(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let stripe = cx.theme().muted;
        if self.entries.is_empty() {
            return div()
                .flex_1()
                .p_4()
                .child("Накоплений по выбранным условиям нет.")
                .into_any_element();
        }
        let page_count = self.page_count();
        self.current_page = self.current_page.min(page_count.saturating_sub(1));
        let current = self.current_page;
        let first = current * SAVINGS_PAGE_SIZE;
        let rows_on_page = (self.entries.len() - first).min(SAVINGS_PAGE_SIZE);
        let total = self.entries.len();

        let pager = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("page-prev")
                    .label("← Назад")
                    .disabled(current == 0)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.current_page = page.current_page.saturating_sub(1);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("page-next")
                    .label("Далее →")
                    .disabled(current + 1 >= page_count)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.current_page += 1;
                        cx.notify();
                    })),
            )
            .child(format!(
                "Страница {} из {} · всего записей: {}",
                current + 1,
                page_count,
                total
            ));
        let header = div()
            .flex()
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_1()
            .border_color(border)
            .font_weight(FontWeight::BOLD)
            .children(
                COLUMN_HEADINGS
                    .into_iter()
                    .zip(COLUMN_WIDTHS)
                    .enumerate()
                    .map(|(index, (title, width))| cell(width, title.into(), border, index > 0)),
            );
        let body = uniform_list(
            "savings-rows",
            rows_on_page,
            cx.processor(move |page, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| page.render_row(first + index, index, stripe, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(pager)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(body),
            )
            .into_any_element()
    }

    fn render_row(
        &mut self,
        entry_index: usize,
        row_index: usize,
        stripe: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let account = self.entries[entry_index].clone();
        let border = cx.theme().border;
        let id = account.id;
        let term = match (account.term_type, account.term_end_date) {
            (SavingsTerm::Limited, Some(date)) => format!("до {}", format_date(date)),
            (SavingsTerm::Limited, None) => "Дата не задана".to_owned(),
            (SavingsTerm::Unlimited, _) => "Бессрочный".to_owned(),
        };
        let dash = || "—".to_owned();
        let values = [
            account.bank_name.clone(),
            account.title.clone(),
            account.amount.to_string(),
            account.currency.map_or_else(dash, |c| c.code().to_owned()),
            format!("{}%", account.base_rate),
            account
                .rate_after_threshold
                .map_or_else(dash, |rate| format!("{rate}%")),
            account
                .threshold_amount
                .map_or_else(dash, |amount| amount.to_string()),
            term,
            account.opened_date.map_or_else(dash, format_date),
            account.interest_accrual_date.map_or_else(dash, format_date),
            account.interest_destination.title().to_owned(),
            if account.is_active {
                "Активно"
            } else {
                "Неактивно"
            }
            .to_owned(),
        ];
        let title = account.title;
        div()
            .id(("savings-row", id as usize))
            .flex()
            .h(px(ROW_HEIGHT))
            .items_center()
            .border_l_1()
            .border_r_1()
            .border_b_1()
            .border_color(border)
            .when(row_index % 2 == 1, |row| row.bg(stripe))
            .children(
                values
                    .into_iter()
                    .zip(COLUMN_WIDTHS)
                    .enumerate()
                    .map(|(index, (value, width))| cell(width, value, border, index > 0)),
            )
            .child(
                div()
                    .w(px(COLUMN_WIDTHS[12]))
                    .h_full()
                    .flex()
                    .items_center()
                    .border_l_1()
                    .border_color(border)
                    .gap_1()
                    .child(
                        Button::new(("edit", id as usize))
                            .compact()
                            .label("Изменить")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.begin_edit(id, window, cx)
                            })),
                    )
                    .child(
                        Button::new(("delete", id as usize))
                            .compact()
                            .label("Удалить")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.confirm_delete(id, title.clone(), window, cx)
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
        .child(text)
}

impl Render for SavingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let success = cx.theme().success;
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let mut root = div().size_full().flex().flex_col().gap_2().p_3();
        root = root.child(
            div().flex().items_center().gap_2().child(
                Button::new("new-savings")
                    .primary()
                    .label("Добавить накопление")
                    .icon(IconName::Plus)
                    .disabled(self.form.is_some() || self.banks.is_empty())
                    .on_click(cx.listener(|page, _, window, cx| page.begin_new(window, cx))),
            ),
        );
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
        if self.form.is_some() {
            if let Some(form) = self.render_form(cx) {
                root = root.child(form);
            }
        } else {
            root = root
                .child(self.render_filters(cx))
                .child(self.render_table(cx));
        }
        root.child(div().text_color(muted).child(FOOTER_NOTE))
            .child(div().text_color(warning).child(FOOTER_WARNING))
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use finance_core::{Currency, InterestDestination, SavingsTerm};

    use super::SavingsForm;

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
        form.opened_date = "2026-03-01".to_owned();
        form.interest_accrual_date = "2026-03-31".to_owned();

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
        form.opened_date = "2026-02-30".to_owned();
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
        form.term_end_date = "2027-03-01".to_owned();
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
        assert_eq!(form.opened_date, "2026-03-01");
        assert_eq!(form.interest_accrual_date, "2026-03-31");

        form.title = "Изменённый вклад".to_owned();
        form.amount = "1250.50".to_owned();
        form.currency = Some(Currency::Rub);
        form.term_type = Some(SavingsTerm::Limited);
        form.term_end_date = "2027-03-01".to_owned();
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
