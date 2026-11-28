//! Детальные отчёты аналитики доходов и расходов на `gpui-kit`.
//!
//! Все запросы выполняются в фоне на отдельном read-only соединении; в UI
//! попадают только уже отфильтрованные и точно агрегированные строки.

use std::path::PathBuf;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use finance_core::{
    AnalysisSlice, AnalysisSliceQuery, AnalysisSliceReport, AnalysisSliceRow, Bank, Category,
    Currency, ExpensePlace, PaymentDocument, PaymentDocumentFilter, RegularExpenseFilter,
    TransactionKind,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{
    Column, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::tooltip::Tooltip as ComponentTooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::StatefulInteractiveElement;
use gpui_kit::*;
use rust_decimal::prelude::ToPrimitive;

use super::helpers::{card_label, expense_place_label, format_currency, DATE_FORMAT};
use super::selectable::{
    selectable_message, selectable_right_aligned_table_cell, selectable_table_cell, selectable_text,
};
use super::support::{
    category_tree_labels, collect_category_descendants, date_period_range, DatePeriod,
    DatePeriodRange,
};

const NONE_LABEL: &str = "Все";
const NO_CARD_LABEL: &str = "Не по карте";
const COLUMN_WIDTHS_KEY: &str = "table_columns.analytics_details.v1";
const HEADERS: [&str; 6] = ["Группа", "Валюта", "Операций", "Сумма", "Доля", "График"];
const DEFAULT_WIDTHS: [f32; 6] = [230.0, 80.0, 100.0, 140.0, 80.0, 250.0];
const MIN_WIDTHS: [f32; 6] = [130.0, 70.0, 85.0, 110.0, 65.0, 120.0];
const COL_GROUP: usize = 0;
const COL_CURRENCY: usize = 1;
const COL_COUNT: usize = 2;
const COL_AMOUNT: usize = 3;
const COL_SHARE: usize = 4;
const COL_CHART: usize = 5;
const REGULAR_OPTIONS: [&str; 3] = ["Все", "Регулярные", "Не регулярные"];
const BAR_COLORS: [u32; 2] = [0x3975a8, 0xbd523c];

type Choice = Entity<SelectState<Vec<SharedString>>>;
type ItemChoice = Entity<SelectState<Vec<FilterItem>>>;

#[derive(Clone)]
struct FilterItem {
    id: usize,
    label: String,
    item_id: &'static str,
}

impl SelectItem for FilterItem {
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
            .id((self.item_id, self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}

fn item_choice(
    items: Vec<String>,
    item_id: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> ItemChoice {
    let items = items
        .into_iter()
        .enumerate()
        .map(|(id, label)| FilterItem { id, label, item_id })
        .collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(0)), window, cx))
}

fn string_choice(items: Vec<String>, window: &mut Window, cx: &mut App) -> Choice {
    let items = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(0)), window, cx))
}

fn selected_date(state: &DatePickerState) -> Option<NaiveDate> {
    match state.date() {
        Date::Single(date) => date,
        Date::Range(_, _) => None,
    }
}

fn payment_filter_from_index(index: usize, card_ids: &[i64]) -> PaymentDocumentFilter {
    match index {
        1 => PaymentDocumentFilter::WithoutCard,
        index => card_ids
            .get(index.wrapping_sub(2))
            .map_or(PaymentDocumentFilter::All, |id| {
                PaymentDocumentFilter::Card(*id)
            }),
    }
}

fn payment_filter_index(filter: PaymentDocumentFilter, card_ids: &[i64]) -> usize {
    match filter {
        PaymentDocumentFilter::All => 0,
        PaymentDocumentFilter::WithoutCard => 1,
        PaymentDocumentFilter::Card(id) => card_ids
            .iter()
            .position(|candidate| *candidate == id)
            .map_or(0, |position| position + 2),
    }
}

fn regular_filter_from_index(index: usize) -> RegularExpenseFilter {
    match index {
        1 => RegularExpenseFilter::Regular,
        2 => RegularExpenseFilter::NotRegular,
        _ => RegularExpenseFilter::All,
    }
}

fn regular_filter_index(filter: RegularExpenseFilter) -> usize {
    match filter {
        RegularExpenseFilter::All => 0,
        RegularExpenseFilter::Regular => 1,
        RegularExpenseFilter::NotRegular => 2,
    }
}

fn currency_from_index(index: usize) -> Option<Currency> {
    match index {
        1 => Some(Currency::Rub),
        2 => Some(Currency::Usd),
        _ => None,
    }
}

fn currency_index(currency: Option<Currency>) -> usize {
    match currency {
        Some(Currency::Rub) => 1,
        Some(Currency::Usd) => 2,
        None => 0,
    }
}

fn cards_for_bank(cards: &[PaymentDocument], bank_id: Option<i64>) -> Vec<&PaymentDocument> {
    cards
        .iter()
        .filter(|card| bank_id.is_none_or(|bank_id| card.bank_id == bank_id))
        .collect()
}

fn payment_cards(cards: &[PaymentDocument], bank_filter: Option<i64>) -> Vec<String> {
    std::iter::once(NONE_LABEL.to_owned())
        .chain(std::iter::once(NO_CARD_LABEL.to_owned()))
        .chain(
            cards_for_bank(cards, bank_filter)
                .into_iter()
                .map(card_label),
        )
        .collect()
}

fn slices_for(kind: TransactionKind) -> Vec<AnalysisSlice> {
    let mut slices = vec![AnalysisSlice::Overall];
    if kind == TransactionKind::Expense {
        slices.push(AnalysisSlice::Places);
    }
    slices.extend([
        AnalysisSlice::Cards,
        AnalysisSlice::Banks,
        AnalysisSlice::Deposits,
        AnalysisSlice::Currency,
    ]);
    slices
}

fn summary_lines(kind: TransactionKind, report: &AnalysisSliceReport) -> Vec<String> {
    let mut lines = report
        .totals
        .iter()
        .map(|total| {
            format!(
                "Итого {}: {} {}; операций: {}",
                total.currency.code(),
                format_currency(&total.amount, 2),
                total.currency.code(),
                total.transaction_count
            )
        })
        .collect::<Vec<_>>();
    if kind == TransactionKind::Expense {
        lines.extend(report.regular_totals.iter().map(|total| {
            format!(
                "Итого по регулярным {}: {} {}; операций: {}",
                total.currency.code(),
                format_currency(&total.amount, 2),
                total.currency.code(),
                total.transaction_count
            )
        }));
    }
    lines
}

fn index_for_slice(kind: TransactionKind, slice: AnalysisSlice) -> usize {
    slices_for(kind)
        .iter()
        .position(|candidate| *candidate == slice)
        .unwrap_or(0)
}

fn format_widths(widths: &[f32]) -> String {
    widths
        .iter()
        .map(|width| width.round().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_widths(value: &str) -> Option<Vec<f32>> {
    let widths = value
        .split(',')
        .map(|part| part.trim().parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()?;
    (widths.len() == HEADERS.len()
        && widths
            .iter()
            .zip(MIN_WIDTHS)
            .all(|(width, minimum)| width.is_finite() && (minimum..=600.0).contains(width)))
    .then_some(widths)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FilterValues {
    search: String,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    category_id: Option<i64>,
    bank_id: Option<i64>,
    card_filter: PaymentDocumentFilter,
    expense_place_id: Option<i64>,
    regular_filter: RegularExpenseFilter,
    currency: Option<Currency>,
}

impl FilterValues {
    fn is_active(&self) -> bool {
        !self.search.is_empty()
            || self.from.is_some()
            || self.to.is_some()
            || self.category_id.is_some()
            || self.bank_id.is_some()
            || self.card_filter.is_active()
            || self.expense_place_id.is_some()
            || self.regular_filter.is_active()
            || self.currency.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestKey {
    slice: AnalysisSlice,
    values: FilterValues,
}

#[derive(Default)]
struct RequestTracker {
    generation: u64,
    applied: Option<RequestKey>,
    loading: bool,
    report: Option<(RequestKey, AnalysisSliceReport)>,
    error: Option<(RequestKey, String)>,
}

impl RequestTracker {
    fn start(&mut self, key: RequestKey) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.applied = Some(key);
        self.loading = true;
        self.error = None;
        self.generation
    }

    fn accept(
        &mut self,
        generation: u64,
        key: RequestKey,
        result: Result<AnalysisSliceReport, String>,
    ) -> bool {
        if generation != self.generation || self.applied.as_ref() != Some(&key) {
            return false;
        }
        self.loading = false;
        match result {
            Ok(report) => self.report = Some((key, report)),
            Err(message) => self.error = Some((key, message)),
        }
        true
    }

    fn current_report(&self) -> Option<&AnalysisSliceReport> {
        let applied = self.applied.as_ref()?;
        if self.error.as_ref().is_some_and(|(key, _)| key == applied) {
            return None;
        }
        self.report
            .as_ref()
            .filter(|(key, _)| key == applied)
            .map(|(_, report)| report)
    }

    fn current_error(&self) -> Option<&str> {
        let applied = self.applied.as_ref()?;
        self.error
            .as_ref()
            .filter(|(key, _)| key == applied)
            .map(|(_, message)| message.as_str())
    }
}

struct AnalysisSliceTable {
    rows: Vec<AnalysisSliceRow>,
    sort: Option<(usize, ColumnSort)>,
    widths: Vec<f32>,
    /// Максимум отдельно по валютам; геометрия полос не означает конвертацию.
    maxima: [f64; 2],
}

impl AnalysisSliceTable {
    fn new(widths: Vec<f32>) -> Self {
        Self {
            rows: Vec::new(),
            sort: Some((COL_AMOUNT, ColumnSort::Descending)),
            widths,
            maxima: [1.0, 1.0],
        }
    }

    fn set_report(&mut self, report: &AnalysisSliceReport) {
        self.rows = report.rows.clone();
        self.maxima = [1.0, 1.0];
        for row in &report.rows {
            let slot = match row.currency {
                Currency::Rub => 0,
                Currency::Usd => 1,
            };
            if let Some(amount) = row.amount.to_f64() {
                self.maxima[slot] = self.maxima[slot].max(amount);
            }
        }
        self.sort_rows();
    }

    fn set_widths(&mut self, widths: &[f32]) {
        self.widths = widths.to_vec();
    }

    fn sort_rows(&mut self) {
        sort_analysis_slice_rows(&mut self.rows, self.sort);
    }
}

fn sort_analysis_slice_rows(rows: &mut [AnalysisSliceRow], sort: Option<(usize, ColumnSort)>) {
    let Some((column, direction)) = sort else {
        rows.sort_by(default_row_order);
        return;
    };
    rows.sort_by(|left, right| {
        let ordering = match column {
            COL_GROUP => left
                .group_name
                .to_lowercase()
                .cmp(&right.group_name.to_lowercase()),
            COL_CURRENCY => left.currency.code().cmp(right.currency.code()),
            COL_COUNT => left.transaction_count.cmp(&right.transaction_count),
            COL_AMOUNT => left.amount.cmp(&right.amount),
            COL_SHARE => left.share_percent.cmp(&right.share_percent),
            _ => std::cmp::Ordering::Equal,
        };
        let ordering = if direction == ColumnSort::Descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| default_row_order(left, right))
    });
}

fn default_row_order(left: &AnalysisSliceRow, right: &AnalysisSliceRow) -> std::cmp::Ordering {
    left.currency
        .code()
        .cmp(right.currency.code())
        .then_with(|| {
            left.group_name
                .to_lowercase()
                .cmp(&right.group_name.to_lowercase())
        })
        .then_with(|| left.group_id.cmp(&right.group_id))
}

fn currency_index_for(currency: Currency) -> usize {
    match currency {
        Currency::Rub => 0,
        Currency::Usd => 1,
    }
}

impl TableDelegate for AnalysisSliceTable {
    fn columns_count(&self, _: &App) -> usize {
        HEADERS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let direction = self
            .sort
            .filter(|(column, _)| *column == col_ix)
            .map_or(ColumnSort::Default, |(_, direction)| direction);
        Column::new(HEADERS[col_ix], HEADERS[col_ix])
            .width(px(self.widths[col_ix]))
            .min_width(px(MIN_WIDTHS[col_ix]))
            .resizable(col_ix != COL_CHART)
            .movable(false)
            .sort(direction)
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.sort = (sort != ColumnSort::Default).then_some((col_ix, sort));
        self.sort_rows();
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
                "analytics-details-empty",
                "По выбранным условиям данных нет.",
            ))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row_ix];
        let row_id = format!("analytics-slice-{}-{}", row.group_id, row.currency.code());
        match col_ix {
            COL_GROUP => {
                selectable_table_cell("analytics-details", &row_id, col_ix, row.group_name.clone())
                    .into_any_element()
            }
            COL_CURRENCY => {
                selectable_table_cell("analytics-details", &row_id, col_ix, row.currency.code())
                    .into_any_element()
            }
            COL_COUNT => selectable_right_aligned_table_cell(
                "analytics-details",
                &row_id,
                col_ix,
                row.transaction_count.to_string(),
            )
            .into_any_element(),
            COL_AMOUNT => selectable_right_aligned_table_cell(
                "analytics-details",
                &row_id,
                col_ix,
                format_currency(&row.amount, 2),
            )
            .into_any_element(),
            COL_SHARE => selectable_right_aligned_table_cell(
                "analytics-details",
                &row_id,
                col_ix,
                format!("{}%", row.share_percent.round_dp(2)),
            )
            .into_any_element(),
            _ => {
                let maximum = self.maxima[currency_index_for(row.currency)];
                let amount = row.amount.to_f64().unwrap_or(0.0);
                let ratio = if maximum > 0.0 {
                    (amount / maximum).clamp(0.005, 1.0)
                } else {
                    0.0
                };
                let width = ratio * 180.0;
                let color = Hsla::from(rgb(BAR_COLORS[currency_index_for(row.currency)]));
                div()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(width as f32))
                            .h(px(12.0))
                            .flex_none()
                            .rounded_sm()
                            .bg(color),
                    )
                    .child(selectable_table_cell(
                        "analytics-details",
                        &row_id,
                        col_ix,
                        row.currency.code(),
                    ))
                    .into_any_element()
            }
        }
    }
}

pub(crate) struct DetailedAnalysisPage {
    kind: TransactionKind,
    database: Rc<Database>,
    database_path: Result<PathBuf, String>,
    slices: Vec<AnalysisSlice>,
    slice: AnalysisSlice,
    /// Pending values for every nested tab; switching tabs preserves them.
    saved: Vec<FilterValues>,
    categories: Vec<Category>,
    banks: Vec<Bank>,
    cards: Vec<PaymentDocument>,
    places: Vec<ExpensePlace>,
    category_ids: Vec<i64>,
    bank_ids: Vec<i64>,
    place_ids: Vec<i64>,
    card_ids: Vec<i64>,
    search: Entity<InputState>,
    from: Entity<DatePickerState>,
    to: Entity<DatePickerState>,
    period_select: Choice,
    category_select: ItemChoice,
    bank_select: ItemChoice,
    card_select: ItemChoice,
    place_select: ItemChoice,
    regular_select: Choice,
    currency_select: Choice,
    tracker: RequestTracker,
    table: Entity<TableState<AnalysisSliceTable>>,
    message: Option<String>,
    restoring: bool,
    _subscriptions: Vec<Subscription>,
}

impl DetailedAnalysisPage {
    pub(crate) fn new(
        kind: TransactionKind,
        database: Rc<Database>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let database_path = match database.path() {
            Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
            _ => Err("Для анализа требуется путь к локальной базе данных.".to_owned()),
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Поиск"));
        let from = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let to = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let period_select = string_choice(
            DatePeriod::ALL
                .into_iter()
                .map(|(_, label)| label.to_owned())
                .collect(),
            window,
            cx,
        );
        let category_select =
            item_choice(vec![NONE_LABEL.to_owned()], "details-category", window, cx);
        let bank_select = item_choice(vec![NONE_LABEL.to_owned()], "details-bank", window, cx);
        let card_select = item_choice(
            vec![NONE_LABEL.to_owned(), NO_CARD_LABEL.to_owned()],
            "details-card",
            window,
            cx,
        );
        let place_select = item_choice(vec![NONE_LABEL.to_owned()], "details-place", window, cx);
        let regular_select = string_choice(
            REGULAR_OPTIONS
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            window,
            cx,
        );
        let currency_select = string_choice(
            std::iter::once(NONE_LABEL.to_owned())
                .chain(
                    Currency::ALL
                        .into_iter()
                        .map(|currency| currency.code().to_owned()),
                )
                .collect(),
            window,
            cx,
        );
        let widths = load_widths(&database);
        let table = cx.new(|cx| {
            TableState::new(AnalysisSliceTable::new(widths), window, cx).col_movable(false)
        });
        let slices = slices_for(kind);
        let saved = vec![FilterValues::default(); slices.len()];
        let mut subscriptions = Vec::new();

        subscriptions.push(cx.subscribe_in(
            &search,
            window,
            |page, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                } else if matches!(event, InputEvent::PressEnter { .. }) {
                    page.apply_filters(cx);
                }
            },
        ));
        for picker in [&from, &to] {
            subscriptions.push(cx.subscribe_in(
                picker,
                window,
                |_, _, _: &DatePickerEvent, _, cx| cx.notify(),
            ));
        }

        let mut page = Self {
            kind,
            database,
            database_path,
            slices,
            slice: AnalysisSlice::Overall,
            saved,
            categories: Vec::new(),
            banks: Vec::new(),
            cards: Vec::new(),
            places: Vec::new(),
            category_ids: Vec::new(),
            bank_ids: Vec::new(),
            place_ids: Vec::new(),
            card_ids: Vec::new(),
            search,
            from,
            to,
            period_select,
            category_select,
            bank_select,
            card_select,
            place_select,
            regular_select,
            currency_select,
            tracker: RequestTracker::default(),
            table,
            message: None,
            restoring: false,
            _subscriptions: subscriptions,
        };
        page.subscribe_controls(window, cx);
        page.reload_sources(window, cx);
        page.apply_filters(cx);
        page
    }
}

impl DetailedAnalysisPage {
    fn subscribe_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.push(cx.subscribe_in(
            &self.category_select,
            window,
            |page: &mut Self, _, event: &SelectEvent<Vec<FilterItem>>, _, cx| {
                if page.restoring {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                let index = value
                    .as_ref()
                    .and_then(|label| page.category_labels().iter().position(|item| item == label));
                page.current_values_mut().category_id = index
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| page.category_ids.get(index).copied());
                page.apply_filters(cx);
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.bank_select,
            window,
            |page: &mut Self, _, event: &SelectEvent<Vec<FilterItem>>, window, cx| {
                if page.restoring {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                let index = value
                    .as_ref()
                    .and_then(|label| page.bank_labels().iter().position(|item| item == label));
                let bank_id = index
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| page.bank_ids.get(index).copied());
                let deposits = page.slice == AnalysisSlice::Deposits;
                let current = page.current_values_mut();
                current.bank_id = bank_id;
                if !deposits {
                    current.card_filter = PaymentDocumentFilter::All;
                }
                page.sync_card_options(window, cx);
                page.apply_filters(cx);
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.card_select,
            window,
            |page: &mut Self, _, event: &SelectEvent<Vec<FilterItem>>, _, cx| {
                if page.restoring {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                let index = value
                    .as_ref()
                    .and_then(|label| page.card_labels().iter().position(|item| item == label));
                let filter = index.map_or(PaymentDocumentFilter::All, |index| {
                    payment_filter_from_index(index, &page.card_ids)
                });
                page.current_values_mut().card_filter = filter;
                page.apply_filters(cx);
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.place_select,
            window,
            |page: &mut Self, _, event: &SelectEvent<Vec<FilterItem>>, _, cx| {
                if page.restoring {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                let index = value
                    .as_ref()
                    .and_then(|label| page.place_labels().iter().position(|item| item == label));
                page.current_values_mut().expense_place_id = index
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| page.place_ids.get(index).copied());
                page.apply_filters(cx);
            },
        ));

        self._subscriptions.push(cx.subscribe_in(
            &self.regular_select,
            window,
            |page: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                if page.restoring {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                let index = value.as_ref().and_then(|label| {
                    REGULAR_OPTIONS
                        .iter()
                        .position(|option| *option == label.as_ref())
                });
                page.current_values_mut().regular_filter =
                    index.map_or(RegularExpenseFilter::All, regular_filter_from_index);
                page.apply_filters(cx);
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.currency_select,
            window,
            |page: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                if page.restoring {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                let index = value.as_ref().and_then(|label| {
                    Currency::ALL
                        .iter()
                        .position(|currency| currency.code() == label.as_ref())
                });
                page.current_values_mut().currency = index.and_then(currency_from_index);
                page.apply_filters(cx);
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.period_select,
            window,
            |page: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if page.restoring {
                    return;
                }
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                if let Some((period, _)) = DatePeriod::ALL
                    .into_iter()
                    .find(|(_, name)| *name == label.as_ref())
                {
                    page.apply_period(period, window, cx);
                }
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.table,
            window,
            |page: &mut Self, _, event: &TableEvent, _, cx| {
                if let TableEvent::ColumnWidthsChanged(widths) = event {
                    page.save_widths(widths, cx);
                }
            },
        ));
    }
}

impl DetailedAnalysisPage {
    fn current_index(&self) -> usize {
        index_for_slice(self.kind, self.slice)
    }

    fn current_values(&self) -> &FilterValues {
        &self.saved[self.current_index()]
    }

    fn current_values_mut(&mut self) -> &mut FilterValues {
        let index = self.current_index();
        &mut self.saved[index]
    }

    fn category_labels(&self) -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(
                category_tree_labels(&self.categories)
                    .into_iter()
                    .map(|(_, label)| label),
            )
            .collect()
    }

    fn bank_labels(&self) -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(self.banks.iter().map(|bank| bank.name.clone()))
            .collect()
    }

    fn card_labels(&self) -> Vec<String> {
        let bank = if self.slice == AnalysisSlice::Deposits {
            None
        } else {
            self.current_values().bank_id
        };
        payment_cards(&self.cards, bank)
    }

    fn place_labels(&self) -> Vec<String> {
        std::iter::once(NONE_LABEL.to_owned())
            .chain(self.places.iter().map(expense_place_label))
            .collect()
    }

    fn reload_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let categories = finance_db::list_categories(&self.database, self.kind, true);
        let banks = finance_db::list_banks(&self.database);
        let cards = finance_db::list_payment_documents(&self.database, None, None, None);
        let places = if self.kind == TransactionKind::Expense {
            finance_db::list_expense_places(&self.database)
        } else {
            Ok(Vec::new())
        };
        match (categories, banks, cards, places) {
            (Ok(categories), Ok(banks), Ok(cards), Ok(places)) => {
                self.categories = categories;
                self.banks = banks;
                self.cards = cards;
                self.places = places;
            }
            (Err(error), _, _, _)
            | (_, Err(error), _, _)
            | (_, _, Err(error), _)
            | (_, _, _, Err(error)) => {
                self.message = Some(format!("Не удалось загрузить справочники: {error}"));
                cx.notify();
                return;
            }
        }
        self.category_ids = category_tree_labels(&self.categories)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        self.bank_ids = self.banks.iter().map(|bank| bank.id).collect();
        self.place_ids = self.places.iter().map(|place| place.id).collect();
        self.replace_items(
            &self.category_select,
            self.category_labels(),
            "details-category",
            window,
            cx,
        );
        self.replace_items(
            &self.bank_select,
            self.bank_labels(),
            "details-bank",
            window,
            cx,
        );
        self.replace_items(
            &self.place_select,
            self.place_labels(),
            "details-place",
            window,
            cx,
        );
        self.sync_card_options(window, cx);
        self.restore_values(window, cx);
    }

    fn replace_items(
        &self,
        select: &ItemChoice,
        labels: Vec<String>,
        item_id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items: Vec<FilterItem> = labels
            .into_iter()
            .enumerate()
            .map(|(id, label)| FilterItem { id, label, item_id })
            .collect();
        select.update(cx, |state, cx| state.set_items(items, window, cx));
    }

    fn sync_card_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.card_ids = cards_for_bank(&self.cards, self.visible_card_bank())
            .into_iter()
            .map(|card| card.id)
            .collect();
        let selection = payment_filter_index(self.current_values().card_filter, &self.card_ids);
        self.replace_items(
            &self.card_select,
            self.card_labels(),
            "details-card",
            window,
            cx,
        );
        self.card_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(selection)), window, cx);
        });
    }

    fn visible_card_bank(&self) -> Option<i64> {
        if self.slice == AnalysisSlice::Deposits {
            None
        } else {
            self.current_values().bank_id
        }
    }
}

impl DetailedAnalysisPage {
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message = None;
        self.reload_sources(window, cx);
        self.apply_filters(cx);
    }

    fn capture_values(&self, cx: &App) -> FilterValues {
        FilterValues {
            search: self.search.read(cx).value().to_string(),
            from: selected_date(self.from.read(cx)),
            to: selected_date(self.to.read(cx)),
            category_id: self.current_values().category_id,
            bank_id: self.current_values().bank_id,
            card_filter: self.current_values().card_filter,
            expense_place_id: self.current_values().expense_place_id,
            regular_filter: self.current_values().regular_filter,
            currency: self.current_values().currency,
        }
    }

    fn restore_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = self.current_values().clone();
        self.restoring = true;
        self.search
            .update(cx, |input, cx| input.set_value(&values.search, window, cx));
        for (picker, date) in [(&self.from, values.from), (&self.to, values.to)] {
            picker.update(cx, |state, cx| {
                state.set_date(Date::Single(date), window, cx)
            });
        }
        self.period_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        let category_row = values
            .category_id
            .and_then(|id| {
                self.category_ids
                    .iter()
                    .position(|candidate| *candidate == id)
            })
            .map_or(0, |position| position + 1);
        let bank_row = values
            .bank_id
            .and_then(|id| self.bank_ids.iter().position(|candidate| *candidate == id))
            .map_or(0, |position| position + 1);
        let place_row = values
            .expense_place_id
            .and_then(|id| self.place_ids.iter().position(|candidate| *candidate == id))
            .map_or(0, |position| position + 1);
        self.sync_card_options(window, cx);
        let card_row = payment_filter_index(values.card_filter, &self.card_ids);
        self.category_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(category_row)), window, cx)
        });
        self.bank_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(bank_row)), window, cx)
        });
        self.card_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(card_row)), window, cx)
        });
        self.place_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(place_row)), window, cx)
        });
        self.regular_select.update(cx, |state, cx| {
            state.set_selected_index(
                Some(IndexPath::default().row(regular_filter_index(values.regular_filter))),
                window,
                cx,
            )
        });
        self.currency_select.update(cx, |state, cx| {
            state.set_selected_index(
                Some(IndexPath::default().row(currency_index(values.currency))),
                window,
                cx,
            )
        });
        self.restoring = false;
    }

    pub(crate) fn set_slice(
        &mut self,
        slice: AnalysisSlice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if slice == self.slice {
            return;
        }
        let captured = self.capture_values(cx);
        *self.current_values_mut() = captured;
        self.slice = slice;
        self.sync_card_options(window, cx);
        self.restore_values(window, cx);
        self.apply_filters(cx);
    }

    fn apply_period(&mut self, period: DatePeriod, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = date_period_range(period, Local::now().date_naive()) else {
            return;
        };
        let (from, to) = match range {
            DatePeriodRange::Unbounded => (None, None),
            DatePeriodRange::Bounded(from, to) => (Some(from), Some(to)),
        };
        self.from.update(cx, |state, cx| {
            state.set_date(Date::Single(from), window, cx)
        });
        self.to
            .update(cx, |state, cx| state.set_date(Date::Single(to), window, cx));
        self.apply_filters(cx);
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        *self.current_values_mut() = FilterValues::default();
        self.restore_values(window, cx);
        self.apply_filters(cx);
    }

    fn clear_filter(&mut self, filter: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        {
            let values = self.current_values_mut();
            match filter {
                "search" => values.search.clear(),
                "from" => values.from = None,
                "to" => values.to = None,
                "category" => values.category_id = None,
                "bank" => values.bank_id = None,
                "card" => values.card_filter = PaymentDocumentFilter::All,
                "place" => values.expense_place_id = None,
                "regular" => values.regular_filter = RegularExpenseFilter::All,
                "currency" => values.currency = None,
                _ => {}
            }
        }
        if filter == "bank" {
            self.current_values_mut().card_filter = PaymentDocumentFilter::All;
            self.sync_card_options(window, cx);
        }
        self.restore_values(window, cx);
        self.apply_filters(cx);
    }
}

impl DetailedAnalysisPage {
    fn query_from(&self, values: &FilterValues) -> AnalysisSliceQuery {
        let deposits = self.slice == AnalysisSlice::Deposits;
        let category_ids = if deposits {
            None
        } else {
            values
                .category_id
                .map(|id| collect_category_descendants(&self.categories, id))
        };
        let place_id = if deposits || self.kind != TransactionKind::Expense {
            None
        } else {
            values.expense_place_id
        };
        let regular = if deposits || self.kind != TransactionKind::Expense {
            RegularExpenseFilter::All
        } else {
            values.regular_filter
        };
        AnalysisSliceQuery {
            kind: self.kind,
            slice: self.slice,
            from: values.from,
            to: values.to,
            category_ids,
            currency: values.currency,
            search: values.search.clone(),
            bank_id: values.bank_id,
            payment_document_filter: values.card_filter,
            expense_place_id: place_id,
            regular_filter: regular,
        }
    }

    fn apply_filters(&mut self, cx: &mut Context<Self>) {
        let values = self.capture_values(cx);
        *self.current_values_mut() = values.clone();
        let key = RequestKey {
            slice: self.slice,
            values: values.clone(),
        };
        let generation = self.tracker.start(key.clone());
        cx.notify();
        if matches!((values.from, values.to), (Some(from), Some(to)) if from > to) {
            self.tracker.accept(
                generation,
                key,
                Err("Начало периода не может быть позже его окончания".to_owned()),
            );
            cx.notify();
            return;
        }
        let path = match &self.database_path {
            Ok(path) => path.clone(),
            Err(error) => {
                self.tracker.accept(generation, key, Err(error.clone()));
                cx.notify();
                return;
            }
        };
        let query = self.query_from(&values);
        let task = cx.background_executor().spawn(async move {
            finance_db::load_analysis_slice_report(&path, &query).map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |page, cx| {
                if page.tracker.accept(generation, key, result) {
                    if let Some(report) = page.tracker.current_report() {
                        page.table.update(cx, |table, cx| {
                            table.delegate_mut().set_report(report);
                            cx.notify();
                        });
                    }
                    page.message = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn save_widths(&mut self, widths: &[Pixels], cx: &mut Context<Self>) {
        let widths = widths
            .iter()
            .map(|width| f32::from(*width))
            .collect::<Vec<_>>();
        if widths.len() != HEADERS.len()
            || !widths
                .iter()
                .zip(MIN_WIDTHS)
                .all(|(width, minimum)| width.is_finite() && (minimum..=600.0).contains(width))
        {
            return;
        }
        self.table
            .update(cx, |table, _| table.delegate_mut().set_widths(&widths));
        if let Err(error) =
            finance_db::set_app_setting(&self.database, COLUMN_WIDTHS_KEY, &format_widths(&widths))
        {
            self.message = Some(format!("Не удалось сохранить ширину колонок: {error}"));
        }
    }
}

fn load_widths(database: &Database) -> Vec<f32> {
    match finance_db::get_app_setting(database, COLUMN_WIDTHS_KEY) {
        Ok(Some(value)) => parse_widths(&value).unwrap_or_else(|| DEFAULT_WIDTHS.to_vec()),
        Ok(None) => DEFAULT_WIDTHS.to_vec(),
        Err(error) => {
            eprintln!("Не удалось прочитать ширину колонок аналитики: {error}");
            DEFAULT_WIDTHS.to_vec()
        }
    }
}

impl DetailedAnalysisPage {
    fn clear_button(
        &self,
        id: &'static str,
        tooltip: &'static str,
        filter: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .compact()
            .small()
            .label("×")
            .tooltip(tooltip)
            .on_click(cx.listener(move |page, _, window, cx| page.clear_filter(filter, window, cx)))
    }

    fn filter_field(
        label: &'static str,
        width: f32,
        disabled: bool,
        clear: Option<Button>,
        control: AnyElement,
    ) -> Div {
        div()
            .w(px(width))
            .flex()
            .flex_col()
            .gap_1()
            .when(disabled, |field| field.opacity(0.5))
            .child(
                div()
                    .flex()
                    .h(px(18.0))
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(12.0)).child(label))
                    .children(clear.map(Button::into_any_element)),
            )
            .child(control)
    }

    fn render_common_filters(&self, cx: &mut Context<Self>) -> Div {
        let values = self.capture_values(cx);
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_2()
            .child(Self::filter_field(
                "Поиск",
                200.0,
                false,
                (!values.search.is_empty()).then(|| {
                    self.clear_button("details-clear-search", "Очистить поиск", "search", cx)
                }),
                Input::new(&self.search)
                    .cleanable(true)
                    .small()
                    .suffix(Icon::new(IconName::Search).small())
                    .into_any_element(),
            ))
            .child(Self::filter_field(
                "С",
                130.0,
                false,
                values.from.is_some().then(|| {
                    self.clear_button("details-clear-from", "Очистить дату «С»", "from", cx)
                }),
                DatePicker::new(&self.from)
                    .placeholder("Выбрать дату")
                    .small()
                    .into_any_element(),
            ))
            .child(Self::filter_field(
                "По",
                130.0,
                false,
                values
                    .to
                    .is_some()
                    .then(|| self.clear_button("details-clear-to", "Очистить дату «По»", "to", cx)),
                DatePicker::new(&self.to)
                    .placeholder("Выбрать дату")
                    .small()
                    .into_any_element(),
            ))
            .child(Self::filter_field(
                "Период",
                150.0,
                false,
                None,
                Select::new(&self.period_select).small().into_any_element(),
            ))
            .child(Self::filter_field(
                "Валюта",
                110.0,
                false,
                values.currency.is_some().then(|| {
                    self.clear_button("details-clear-currency", "Очистить валюту", "currency", cx)
                }),
                Select::new(&self.currency_select)
                    .small()
                    .into_any_element(),
            ))
    }
}

impl DetailedAnalysisPage {
    fn render_entity_filters(&self, cx: &mut Context<Self>) -> Div {
        let values = self.capture_values(cx);
        let deposits = self.slice == AnalysisSlice::Deposits;
        let expense = self.kind == TransactionKind::Expense;
        let payment_label = if expense {
            "Способ оплаты"
        } else {
            "Способ получения"
        };
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_2()
            .child(Self::filter_field(
                "Категория",
                230.0,
                deposits,
                (!deposits && values.category_id.is_some()).then(|| {
                    self.clear_button(
                        "details-clear-category",
                        "Очистить категорию",
                        "category",
                        cx,
                    )
                }),
                Select::new(&self.category_select)
                    .small()
                    .disabled(deposits)
                    .into_any_element(),
            ))
            .child(Self::filter_field(
                "Банк",
                190.0,
                false,
                values
                    .bank_id
                    .is_some()
                    .then(|| self.clear_button("details-clear-bank", "Очистить банк", "bank", cx)),
                Select::new(&self.bank_select).small().into_any_element(),
            ))
            .child(Self::filter_field(
                payment_label,
                220.0,
                false,
                values.card_filter.is_active().then(|| {
                    self.clear_button("details-clear-card", "Очистить способ", "card", cx)
                }),
                Select::new(&self.card_select).small().into_any_element(),
            ))
            .child(Self::filter_field(
                "Место покупки",
                190.0,
                !expense || deposits,
                (!deposits && expense && values.expense_place_id.is_some()).then(|| {
                    self.clear_button("details-clear-place", "Очистить место", "place", cx)
                }),
                Select::new(&self.place_select)
                    .small()
                    .disabled(!expense || deposits)
                    .into_any_element(),
            ))
            .child(Self::filter_field(
                "Регулярные расходы",
                170.0,
                !expense || deposits,
                (!deposits && expense && values.regular_filter.is_active()).then(|| {
                    self.clear_button(
                        "details-clear-regular",
                        "Очистить регулярность",
                        "regular",
                        cx,
                    )
                }),
                Select::new(&self.regular_select)
                    .small()
                    .disabled(!expense || deposits)
                    .into_any_element(),
            ))
    }

    fn render_filters(&mut self, cx: &mut Context<Self>) -> Div {
        let active = self.current_values().is_active();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.render_common_filters(cx))
            .child(self.render_entity_filters(cx))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("details-apply-filters")
                            .primary()
                            .small()
                            .label("Применить")
                            .on_click(cx.listener(|page, _, _, cx| page.apply_filters(cx))),
                    )
                    .child(
                        Button::new("details-reset-filters")
                            .small()
                            .label("Сбросить фильтры")
                            .disabled(!active)
                            .on_click(
                                cx.listener(|page, _, window, cx| page.reset_filters(window, cx)),
                            ),
                    ),
            )
    }
}

impl Render for DetailedAnalysisPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let selected = self.current_index();
        let title = if self.kind == TransactionKind::Income {
            "Аналитика доходов"
        } else {
            "Аналитика расходов"
        };
        let tabs = TabBar::new("analysis-details-tabs")
            .selected_index(selected)
            .on_click(cx.listener(|page: &mut Self, index: &usize, window, cx| {
                if let Some(slice) = page.slices.get(*index).copied() {
                    page.set_slice(slice, window, cx);
                }
            }))
            .children(
                self.slices
                    .iter()
                    .map(|slice| Tab::new().label(slice.title())),
            );
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(
                div()
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
                        Button::new("analysis-details-refresh")
                            .secondary()
                            .large()
                            .label("Обновить")
                            .icon(IconName::Replace)
                            .on_click(cx.listener(|page, _, window, cx| page.reload(window, cx))),
                    ),
            )
            .child(tabs)
            .child(self.render_filters(cx));
        if let Some(message) = &self.message {
            root = root.child(selectable_message(
                "analysis-details-directory-error",
                message.clone(),
                danger,
            ));
        }
        if let Some(error) = self.tracker.current_error() {
            root = root.child(selectable_message(
                "analysis-details-error",
                error.to_owned(),
                danger,
            ));
        }
        let report = self.tracker.current_report().cloned();
        let summaries = report
            .as_ref()
            .map(|report| summary_lines(self.kind, report))
            .unwrap_or_default();
        let body = if report.is_some() {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_x_scrollbar()
                        .child(DataTable::new(&self.table).stripe(true).bordered(true)),
                )
                .into_any_element()
        } else if self.tracker.loading {
            div().flex_1().child("Загружаем отчёт…").into_any_element()
        } else {
            div().flex_1().into_any_element()
        };
        let mut footer = div().flex().flex_col().gap_1().children(
            summaries
                .into_iter()
                .enumerate()
                .map(|(index, line)| selectable_text(("analysis-details-total", index), line)),
        );
        if self.slice == AnalysisSlice::Deposits {
            let meaning = if self.kind == TransactionKind::Income {
                "Пополнения вкладов по журналу переводов: это не доходы и не проценты."
            } else {
                "Снятия со вкладов по журналу переводов: это не расходы и не остаток вклада."
            };
            footer = footer.child(selectable_message(
                "analysis-deposits-note",
                format!(
                    "{meaning} Учитываются только записи журнала, включая переводы между вкладами; начальные остатки и выплаты процентов сюда не входят."
                ),
                cx.theme().warning,
            ));
        }
        root.child(body)
            .child(footer)
            .child(selectable_text(
                "analysis-details-share-note",
                "Доли рассчитываются отдельно внутри каждой валюты. RUB и USD не конвертируются; длина полос показывает только долю внутри своей валюты.",
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        currency_index, index_for_slice, parse_widths, payment_filter_from_index,
        payment_filter_index, regular_filter_index, slices_for, summary_lines, AnalysisSliceTable,
        RequestTracker, COL_AMOUNT, COL_GROUP, DEFAULT_WIDTHS, HEADERS,
    };
    use finance_core::{
        AnalysisSlice, AnalysisSliceItem, AnalysisSliceReport, AnalysisSliceRow, Currency,
        PaymentDocumentFilter, RegularExpenseFilter, TransactionKind,
    };
    use gpui_kit::component::table::ColumnSort;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn request_values(slice: AnalysisSlice, label: &str) -> super::RequestKey {
        super::RequestKey {
            slice,
            values: super::FilterValues {
                search: label.to_owned(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn nested_tabs_follow_the_planned_order_for_each_kind() {
        assert_eq!(
            slices_for(TransactionKind::Income)
                .into_iter()
                .map(|slice| slice.title())
                .collect::<Vec<_>>(),
            [
                "Общая аналитика",
                "По карточкам",
                "По банкам",
                "По вкладам",
                "По валюте",
            ]
        );
        assert_eq!(
            slices_for(TransactionKind::Expense)
                .into_iter()
                .map(|slice| slice.title())
                .collect::<Vec<_>>(),
            [
                "Общая аналитика",
                "По местам трат",
                "По карточкам",
                "По банкам",
                "По вкладам",
                "По валюте",
            ]
        );
        assert_eq!(
            index_for_slice(TransactionKind::Expense, AnalysisSlice::Deposits),
            4
        );
    }

    #[test]
    fn payment_and_regular_filter_indices_round_trip() {
        let ids = [7, 9];
        assert_eq!(
            payment_filter_from_index(
                payment_filter_index(PaymentDocumentFilter::Card(9), &ids),
                &ids
            ),
            PaymentDocumentFilter::Card(9)
        );
        assert_eq!(
            payment_filter_from_index(
                payment_filter_index(PaymentDocumentFilter::WithoutCard, &ids),
                &ids
            ),
            PaymentDocumentFilter::WithoutCard
        );
        assert_eq!(regular_filter_index(RegularExpenseFilter::NotRegular), 2);
        assert_eq!(currency_index(Some(Currency::Usd)), 2);
    }

    #[test]
    fn expense_footer_places_regular_totals_after_all_totals() {
        let report = AnalysisSliceReport {
            rows: Vec::new(),
            totals: vec![
                finance_core::AnalysisSliceTotal {
                    currency: Currency::Rub,
                    amount: Decimal::from_str("1234.50").unwrap(),
                    transaction_count: 2,
                },
                finance_core::AnalysisSliceTotal {
                    currency: Currency::Usd,
                    amount: Decimal::from(5),
                    transaction_count: 1,
                },
            ],
            regular_totals: vec![finance_core::AnalysisSliceTotal {
                currency: Currency::Rub,
                amount: Decimal::from(99),
                transaction_count: 1,
            }],
        };

        assert_eq!(
            summary_lines(TransactionKind::Expense, &report),
            [
                "Итого RUB: 1 234.50 RUB; операций: 2",
                "Итого USD: 5.00 USD; операций: 1",
                "Итого по регулярным RUB: 99.00 RUB; операций: 1",
            ]
        );
    }

    #[test]
    fn income_footer_does_not_show_regular_totals() {
        let report = AnalysisSliceReport {
            rows: Vec::new(),
            totals: Vec::new(),
            regular_totals: vec![finance_core::AnalysisSliceTotal {
                currency: Currency::Rub,
                amount: Decimal::from(99),
                transaction_count: 1,
            }],
        };

        assert!(summary_lines(TransactionKind::Income, &report).is_empty());
    }

    #[test]
    fn corrupt_saved_widths_are_rejected() {
        assert_eq!(
            parse_widths(&super::format_widths(&DEFAULT_WIDTHS)),
            Some(DEFAULT_WIDTHS.to_vec())
        );
        assert_eq!(parse_widths("100,NaN,100,100,100,100"), None);
        assert_eq!(parse_widths("100,100"), None);
        assert_eq!(HEADERS.len(), DEFAULT_WIDTHS.len());
    }

    #[test]
    fn stale_result_does_not_replace_current_report() {
        let mut tracker = RequestTracker::default();
        let first = request_values(AnalysisSlice::Overall, "first");
        let second = request_values(AnalysisSlice::Cards, "second");
        let first_generation = tracker.start(first.clone());
        let second_generation = tracker.start(second.clone());
        let report = finance_core::summarize_analysis_slice(&[AnalysisSliceItem {
            group_id: 1,
            group_name: "Группа".to_owned(),
            currency: Currency::Rub,
            amount: Decimal::from(5),
        }])
        .unwrap();

        assert!(!tracker.accept(first_generation, first, Ok(report.clone())));
        assert!(tracker.accept(second_generation, second, Ok(report)));
        assert_eq!(tracker.current_report().unwrap().rows.len(), 1);
    }

    #[test]
    fn table_sort_uses_exact_amounts_and_keeps_bars_aligned_with_rows() {
        let mut table = AnalysisSliceTable::new(DEFAULT_WIDTHS.to_vec());
        let rows = vec![
            AnalysisSliceRow {
                group_id: 1,
                group_name: "Меньше".to_owned(),
                currency: Currency::Rub,
                amount: Decimal::from_str("1.25").unwrap(),
                transaction_count: 1,
                share_percent: Decimal::from(25),
            },
            AnalysisSliceRow {
                group_id: 2,
                group_name: "Больше".to_owned(),
                currency: Currency::Rub,
                amount: Decimal::from_str("3.75").unwrap(),
                transaction_count: 1,
                share_percent: Decimal::from(75),
            },
        ];
        let report = finance_core::AnalysisSliceReport {
            rows,
            totals: vec![finance_core::AnalysisSliceTotal {
                currency: Currency::Rub,
                amount: Decimal::from(5),
                transaction_count: 2,
            }],
            regular_totals: Vec::new(),
        };
        let mut sorted = report.rows.clone();
        super::sort_analysis_slice_rows(&mut sorted, Some((COL_AMOUNT, ColumnSort::Descending)));
        assert_eq!(sorted[0].group_name, "Больше");
        super::sort_analysis_slice_rows(&mut sorted, Some((COL_GROUP, ColumnSort::Ascending)));
        assert_eq!(sorted[0].group_name, "Больше");
        table.set_report(&report);
        assert_eq!(table.rows.len(), 2);
    }
}
