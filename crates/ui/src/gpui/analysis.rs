//! Страница «Анализ» на `gpui-kit`. Поведение соответствует `analysis_page.rs`:
//! фильтры применяются явно, отчёт читается в фоне отдельным соединением, а
//! устаревшие результаты отбрасываются счётчиком поколений. Расчёты выполняют
//! `finance-core` и `finance-db`; диаграммы рисуются средствами `gpui`.

use std::path::PathBuf;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use finance_core::{
    summarize_analysis, summarize_transaction_balance, AnalysisCategoryTotal, AnalysisRegularTotal,
    AnalysisSummary, AnalysisTotal, Category, Currency, TransactionAmount, TransactionBalanceError,
    TransactionKind,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip as ComponentTooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::StatefulInteractiveElement;
use gpui_kit::*;
use rust_decimal::prelude::ToPrimitive;

use super::helpers::{format_currency, DATE_FORMAT};
use super::selectable::{
    selectable_message, selectable_right_aligned_table_cell, selectable_table_cell, selectable_text,
};

const NONE_LABEL: &str = "Все";
const ROW_HEIGHT: f32 = 30.0;
const PIE_CHART_DIAMETER: f32 = 120.0;
const INCOME_COLUMN_HEADERS: [&str; 4] = ["Категория", "Операций", "Сумма", "Доля"];
const EXPENSE_COLUMN_HEADERS: [&str; 5] = ["Категория", "Операций", "Сумма", "Доля", "Регулярные"];
const COLUMN_WIDTHS: [f32; 5] = [205.0, 95.0, 135.0, 70.0, 95.0];
const MIN_COLUMN_WIDTHS: [f32; 5] = [140.0, 90.0, 115.0, 65.0, 90.0];
const COL_CATEGORY: usize = 0;
const COL_TRANSACTION_COUNT: usize = 1;
const COL_AMOUNT: usize = 2;
const COL_SHARE: usize = 3;
const COL_REGULAR: usize = 4;
const PIE_CHART_COLORS_DARK: [u32; 8] = [
    0x4e7db1, 0xcd6e5c, 0x379291, 0xcfb171, 0x7b5da0, 0xd79669, 0x61855b, 0xad5d76,
];
const PIE_CHART_COLORS_LIGHT: [u32; 8] = [
    0x3975a8, 0xbd523c, 0x247e75, 0xa47b22, 0x78539b, 0xbe692c, 0x5a7d3c, 0xaa4c67,
];

type Choice = Entity<SelectState<Vec<SharedString>>>;
type CategoryChoice = Entity<SelectState<Vec<CategoryFilterItem>>>;

const INCOME_CATEGORY_FILTER_ITEM_ID: &str = "analysis-income-category-filter-item";
const EXPENSE_CATEGORY_FILTER_ITEM_ID: &str = "analysis-expense-category-filter-item";

#[derive(Clone)]
struct CategoryFilterItem {
    id: usize,
    label: String,
    item_id: &'static str,
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
            .id((self.item_id, self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalysisFilters {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    income_category_id: Option<i64>,
    expense_category_id: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedFilters {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    income_category_id: Option<i64>,
    expense_category_id: Option<i64>,
}

fn parse_filters(filters: &AnalysisFilters) -> Result<ParsedFilters, String> {
    let from = filters.from;
    let to = filters.to;
    if matches!((from, to), (Some(from), Some(to)) if from > to) {
        return Err("Начало периода не может быть позже его окончания".to_owned());
    }
    Ok(ParsedFilters {
        from,
        to,
        income_category_id: filters.income_category_id,
        expense_category_id: filters.expense_category_id,
    })
}

fn selected_date(state: &DatePickerState) -> Option<NaiveDate> {
    match state.date() {
        Date::Single(date) => date,
        Date::Range(_, _) => None,
    }
}

/// Состояние запросов: номер поколения защищает от записи устаревших результатов.
#[derive(Default)]
struct RequestTracker {
    generation: u64,
    applied: Option<AnalysisFilters>,
    loading: bool,
    report: Option<(AnalysisFilters, AnalysisSummary)>,
    error: Option<(AnalysisFilters, String)>,
}

impl RequestTracker {
    /// Начинает новый запрос и возвращает его поколение.
    fn start(&mut self, filters: AnalysisFilters) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.applied = Some(filters);
        self.loading = true;
        self.error = None;
        self.generation
    }

    /// Принимает результат, только если он относится к актуальному запросу.
    fn accept(
        &mut self,
        generation: u64,
        filters: AnalysisFilters,
        result: Result<AnalysisSummary, String>,
    ) -> bool {
        if generation != self.generation || self.applied.as_ref() != Some(&filters) {
            return false;
        }
        self.loading = false;
        match result {
            Ok(summary) => self.report = Some((filters, summary)),
            Err(error) => self.error = Some((filters, error)),
        }
        true
    }

    fn current_error(&self) -> Option<&str> {
        let applied = self.applied.as_ref()?;
        self.error
            .as_ref()
            .filter(|(filters, _)| filters == applied)
            .map(|(_, message)| message.as_str())
    }

    fn current_report(&self) -> Option<&AnalysisSummary> {
        let applied = self.applied.as_ref()?;
        if self.current_error().is_some() {
            return None;
        }
        self.report
            .as_ref()
            .filter(|(filters, _)| filters == applied)
            .map(|(_, summary)| summary)
    }
}

#[derive(Clone)]
struct PieChartSlice<'a> {
    category: &'a AnalysisCategoryTotal,
    share: f32,
}

struct AnalysisTable {
    kind: TransactionKind,
    currency: Currency,
    rows: Vec<AnalysisCategoryTotal>,
    total: Option<AnalysisTotal>,
    regular_total: Option<AnalysisRegularTotal>,
    sort: Option<(usize, ColumnSort)>,
}

impl AnalysisTable {
    fn new(kind: TransactionKind, currency: Currency) -> Self {
        Self {
            kind,
            currency,
            rows: Vec::new(),
            total: None,
            regular_total: None,
            sort: None,
        }
    }

    fn set_summary(&mut self, summary: &AnalysisSummary) {
        self.rows = summary
            .categories
            .iter()
            .filter(|category| category.kind == self.kind && category.currency == self.currency)
            .cloned()
            .collect();
        self.total = summary
            .totals
            .iter()
            .find(|total| total.kind == self.kind && total.currency == self.currency)
            .cloned();
        self.regular_total = self.total.as_ref().and_then(|total| total.regular.clone());
        sort_analysis_categories(&mut self.rows, self.sort);
    }
}

fn default_analysis_category_order(
    a: &AnalysisCategoryTotal,
    b: &AnalysisCategoryTotal,
) -> std::cmp::Ordering {
    a.category_name
        .to_lowercase()
        .cmp(&b.category_name.to_lowercase())
        .then(a.category_id.cmp(&b.category_id))
}

fn sort_analysis_categories(rows: &mut [AnalysisCategoryTotal], sort: Option<(usize, ColumnSort)>) {
    let Some((column, direction)) = sort else {
        rows.sort_by(default_analysis_category_order);
        return;
    };
    rows.sort_by(|a, b| {
        let ordering = match column {
            COL_CATEGORY => a
                .category_name
                .to_lowercase()
                .cmp(&b.category_name.to_lowercase()),
            COL_TRANSACTION_COUNT => a.transaction_count.cmp(&b.transaction_count),
            COL_AMOUNT => a.amount.cmp(&b.amount),
            COL_SHARE => a.share_percent.cmp(&b.share_percent),
            COL_REGULAR => a.is_regular.cmp(&b.is_regular),
            _ => std::cmp::Ordering::Equal,
        };
        let ordering = if direction == ColumnSort::Descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| default_analysis_category_order(a, b))
    });
}

fn analysis_table_index(kind: TransactionKind, currency: Currency) -> usize {
    match (kind, currency) {
        (TransactionKind::Income, Currency::Rub) => 0,
        (TransactionKind::Income, Currency::Usd) => 1,
        (TransactionKind::Expense, Currency::Rub) => 2,
        (TransactionKind::Expense, Currency::Usd) => 3,
    }
}

fn analysis_column_headers(kind: TransactionKind) -> &'static [&'static str] {
    match kind {
        TransactionKind::Income => &INCOME_COLUMN_HEADERS,
        TransactionKind::Expense => &EXPENSE_COLUMN_HEADERS,
    }
}

fn analysis_table_width(kind: TransactionKind) -> f32 {
    analysis_column_headers(kind)
        .iter()
        .enumerate()
        .map(|(index, _)| COLUMN_WIDTHS[index])
        .sum()
}

/// Values for the second footer row; income tables never show it.
fn regular_summary_values(kind: TransactionKind, total: &AnalysisTotal) -> Option<[String; 5]> {
    if kind != TransactionKind::Expense {
        return None;
    }
    let regular = total.regular.as_ref()?;
    Some([
        "Итого в регулярных расходах".to_owned(),
        regular.transaction_count.to_string(),
        format_currency(&regular.amount, 2),
        format!("{}%", regular.share_percent.round_dp(2)),
        "—".to_owned(),
    ])
}

fn regular_label(is_regular: bool) -> &'static str {
    if is_regular {
        "Да"
    } else {
        "Нет"
    }
}

impl TableDelegate for AnalysisTable {
    fn columns_count(&self, _: &App) -> usize {
        analysis_column_headers(self.kind).len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let direction = self
            .sort
            .filter(|(sorted, _)| *sorted == col_ix)
            .map_or(ColumnSort::Default, |(_, direction)| direction);
        let headers = analysis_column_headers(self.kind);
        let column = Column::new(headers[col_ix], headers[col_ix])
            .width(px(COLUMN_WIDTHS[col_ix]))
            .min_width(px(MIN_COLUMN_WIDTHS[col_ix]))
            .resizable(false)
            .movable(false)
            .sort(direction);
        if col_ix == COL_CATEGORY || col_ix == COL_REGULAR {
            column
        } else {
            column.text_right()
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
        sort_analysis_categories(&mut self.rows, self.sort);
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
                (
                    "analysis-empty",
                    analysis_table_index(self.kind, self.currency),
                ),
                "Операций по выбранным условиям нет.",
            ))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let category = &self.rows[row_ix];
        let row_id = format!(
            "{}-{}",
            analysis_table_index(self.kind, self.currency),
            category.category_id
        );
        match col_ix {
            COL_CATEGORY => div()
                .w_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(selectable_table_cell(
                    "analysis",
                    &row_id,
                    col_ix,
                    category.category_name.clone(),
                ))
                .into_any_element(),
            COL_TRANSACTION_COUNT => selectable_right_aligned_table_cell(
                "analysis",
                &row_id,
                col_ix,
                category.transaction_count.to_string(),
            )
            .into_any_element(),
            COL_AMOUNT => selectable_right_aligned_table_cell(
                "analysis",
                &row_id,
                col_ix,
                format_currency(&category.amount, 2),
            )
            .into_any_element(),
            COL_SHARE => selectable_right_aligned_table_cell(
                "analysis",
                &row_id,
                col_ix,
                format!("{}%", category.share_percent.round_dp(2)),
            )
            .into_any_element(),
            COL_REGULAR => selectable_table_cell(
                "analysis",
                &row_id,
                col_ix,
                regular_label(category.is_regular),
            )
            .into_any_element(),
            _ => div().into_any_element(),
        }
    }
}

fn pie_chart_slices(
    summary: &AnalysisSummary,
    kind: TransactionKind,
    currency: Currency,
) -> Option<Vec<PieChartSlice<'_>>> {
    summary
        .categories
        .iter()
        .filter(|category| category.kind == kind && category.currency == currency)
        .map(|category| {
            let share = category.share_percent.to_f32()?;
            if !share.is_finite() || share < 0.0 {
                return None;
            }
            Some(PieChartSlice { category, share })
        })
        .collect()
}

fn pie_chart_color(category_id: i64, is_dark: bool) -> Hsla {
    let colors = if is_dark {
        &PIE_CHART_COLORS_DARK
    } else {
        &PIE_CHART_COLORS_LIGHT
    };
    let index = category_id.rem_euclid(colors.len() as i64) as usize;
    Hsla::from(rgb(colors[index]))
}

/// Вершины сектора: центр и дуга, координаты относительно левого верхнего угла.
fn pie_slice_points(
    center: (f32, f32),
    radius: f32,
    start_angle: f32,
    end_angle: f32,
) -> Vec<(f32, f32)> {
    let segments = ((end_angle - start_angle).abs() * 16.0).ceil().max(1.0) as usize;
    let mut points = Vec::with_capacity(segments + 2);
    points.push(center);
    for segment in 0..=segments {
        let fraction = segment as f32 / segments as f32;
        let angle = start_angle + (end_angle - start_angle) * fraction;
        points.push((
            center.0 + angle.cos() * radius,
            center.1 + angle.sin() * radius,
        ));
    }
    points
}

fn pie_chart(slices: Vec<(i64, f32)>, is_dark: bool) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let total: f32 = slices.iter().map(|(_, share)| share).sum();
            if !total.is_finite() || total <= 0.0 {
                return;
            }
            let side = bounds.size.width.min(bounds.size.height).as_f32();
            let center = (side / 2.0, side / 2.0);
            let radius = side / 2.0 - 2.0;
            let mut start = -std::f32::consts::FRAC_PI_2;
            for (category_id, share) in &slices {
                if *share <= 0.0 {
                    continue;
                }
                let end = start + std::f32::consts::TAU * share / total;
                let mut builder = PathBuilder::fill();
                for (index, (x, y)) in pie_slice_points(center, radius, start, end)
                    .into_iter()
                    .enumerate()
                {
                    let position = point(bounds.origin.x + px(x), bounds.origin.y + px(y));
                    if index == 0 {
                        builder.move_to(position);
                    } else {
                        builder.line_to(position);
                    }
                }
                builder.close();
                // Построение выпуклого многоугольника из конечных точек не должно
                // завершаться ошибкой; при сбое сектор не рисуется, а таблица
                // и легенда остаются источником данных.
                if let Ok(path) = builder.build() {
                    window.paint_path(path, pie_chart_color(*category_id, is_dark));
                }
                start = end;
            }
        },
    )
    .size(px(PIE_CHART_DIAMETER))
    .flex_none()
}

/// Выбор категории с фиксированным списком «Все» + дерево категорий.
struct CategoryPicker {
    categories: Vec<Category>,
    ids: Vec<i64>,
    labels: Vec<String>,
    selected: Option<i64>,
    select: CategoryChoice,
}

impl CategoryPicker {
    fn labels_for(categories: &[Category]) -> (Vec<i64>, Vec<String>) {
        let tree = category_tree_labels(categories);
        let ids = tree.iter().map(|(id, _)| *id).collect();
        let labels = std::iter::once(NONE_LABEL.to_owned())
            .chain(tree.into_iter().map(|(_, label)| label))
            .collect();
        (ids, labels)
    }

    fn handle_confirm(&mut self, value: &Option<String>) {
        self.selected = value
            .as_ref()
            .and_then(|label| self.labels.iter().position(|item| item == label))
            .and_then(|position| position.checked_sub(1))
            .and_then(|position| self.ids.get(position).copied());
    }
}

pub(crate) struct AnalysisPage {
    database: Rc<Database>,
    database_path: Result<PathBuf, String>,
    from: Entity<DatePickerState>,
    to: Entity<DatePickerState>,
    period_select: Choice,
    income: CategoryPicker,
    expense: CategoryPicker,
    tracker: RequestTracker,
    tables: [Entity<TableState<AnalysisTable>>; 4],
    message: Option<String>,
    _subscriptions: Vec<Subscription>,
}

fn choice(items: Vec<String>, window: &mut Window, cx: &mut Context<AnalysisPage>) -> Choice {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(0)), window, cx))
}

fn category_choice(
    items: Vec<String>,
    item_id: &'static str,
    window: &mut Window,
    cx: &mut Context<AnalysisPage>,
) -> CategoryChoice {
    let items: Vec<CategoryFilterItem> = items
        .into_iter()
        .enumerate()
        .map(|(id, label)| CategoryFilterItem { id, label, item_id })
        .collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(0)), window, cx))
}

impl AnalysisPage {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let database_path = match database.path() {
            Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
            _ => Err("Для анализа требуется путь к локальной базе данных.".to_owned()),
        };
        let from = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let to = cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT));
        let period_labels = super::support::DatePeriod::ALL
            .into_iter()
            .map(|(_, name)| name.to_owned())
            .collect();
        let period_select = choice(period_labels, window, cx);
        let income_select = category_choice(
            vec![NONE_LABEL.to_owned()],
            INCOME_CATEGORY_FILTER_ITEM_ID,
            window,
            cx,
        );
        let expense_select = category_choice(
            vec![NONE_LABEL.to_owned()],
            EXPENSE_CATEGORY_FILTER_ITEM_ID,
            window,
            cx,
        );

        let mut subscriptions = Vec::new();
        for date_picker in [&from, &to] {
            subscriptions.push(cx.subscribe_in(
                date_picker,
                window,
                |_, _, _: &DatePickerEvent, _, cx| cx.notify(),
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &period_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                if let Some((period, _)) = super::support::DatePeriod::ALL
                    .into_iter()
                    .find(|(_, name)| *name == label.as_ref())
                {
                    this.set_period(period, window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &income_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<CategoryFilterItem>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.income.handle_confirm(value);
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &expense_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<CategoryFilterItem>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.expense.handle_confirm(value);
                cx.notify();
            },
        ));

        let picker = |select: CategoryChoice| CategoryPicker {
            categories: Vec::new(),
            ids: Vec::new(),
            labels: vec![NONE_LABEL.to_owned()],
            selected: None,
            select,
        };
        let tables = [
            (TransactionKind::Income, Currency::Rub),
            (TransactionKind::Income, Currency::Usd),
            (TransactionKind::Expense, Currency::Rub),
            (TransactionKind::Expense, Currency::Usd),
        ]
        .map(|(kind, currency)| {
            cx.new(|cx| {
                TableState::new(AnalysisTable::new(kind, currency), window, cx).col_movable(false)
            })
        });
        let mut page = Self {
            database,
            database_path,
            from,
            to,
            period_select,
            income: picker(income_select),
            expense: picker(expense_select),
            tracker: RequestTracker::default(),
            tables,
            message: None,
            _subscriptions: subscriptions,
        };
        page.reload(window, cx);
        page
    }

    /// Обновляет списки категорий и перечитывает отчёт по текущим фильтрам.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message = None;
        for kind in [TransactionKind::Income, TransactionKind::Expense] {
            match finance_db::list_categories(&self.database, kind, true) {
                Ok(categories) => {
                    let item_id = match kind {
                        TransactionKind::Income => INCOME_CATEGORY_FILTER_ITEM_ID,
                        TransactionKind::Expense => EXPENSE_CATEGORY_FILTER_ITEM_ID,
                    };
                    let picker = match kind {
                        TransactionKind::Income => &mut self.income,
                        TransactionKind::Expense => &mut self.expense,
                    };
                    let (ids, labels) = CategoryPicker::labels_for(&categories);
                    // Удалённая категория сбрасывается на «Все».
                    if picker.selected.is_some_and(|id| !ids.contains(&id)) {
                        picker.selected = None;
                    }
                    let row = picker
                        .selected
                        .and_then(|id| ids.iter().position(|candidate| *candidate == id))
                        .map_or(0, |position| position + 1);
                    picker.categories = categories;
                    picker.ids = ids;
                    picker.labels = labels.clone();
                    let items: Vec<CategoryFilterItem> = labels
                        .into_iter()
                        .enumerate()
                        .map(|(id, label)| CategoryFilterItem { id, label, item_id })
                        .collect();
                    picker.select.update(cx, |state, cx| {
                        state.set_items(items, window, cx);
                        state.set_selected_index(Some(IndexPath::default().row(row)), window, cx);
                    });
                }
                Err(error) => {
                    self.message = Some(error.to_string());
                }
            }
        }
        self.apply_pending_filters(cx);
    }

    fn filters(&self, cx: &App) -> AnalysisFilters {
        AnalysisFilters {
            from: selected_date(self.from.read(cx)),
            to: selected_date(self.to.read(cx)),
            income_category_id: self.income.selected,
            expense_category_id: self.expense.selected,
        }
    }

    fn has_active_filters(&self, cx: &App) -> bool {
        let filters = self.filters(cx);
        filters.from.is_some()
            || filters.to.is_some()
            || filters.income_category_id.is_some()
            || filters.expense_category_id.is_some()
    }

    fn update_tables(&self, summary: &AnalysisSummary, cx: &mut Context<Self>) {
        for table in &self.tables {
            table.update(cx, |table, cx| {
                table.delegate_mut().set_summary(summary);
                cx.notify();
            });
        }
    }

    fn set_period(
        &mut self,
        period: super::support::DatePeriod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = super::support::date_period_range(period, Local::now().date_naive())
        else {
            return;
        };
        let (from, to) = match range {
            super::support::DatePeriodRange::Unbounded => (None, None),
            super::support::DatePeriodRange::Bounded(from, to) => (Some(from), Some(to)),
        };
        self.from.update(cx, |state, cx| {
            state.set_date(Date::Single(from), window, cx)
        });
        self.to
            .update(cx, |state, cx| state.set_date(Date::Single(to), window, cx));
        cx.notify();
    }

    fn apply_pending_filters(&mut self, cx: &mut Context<Self>) {
        let filters = self.filters(cx);
        let generation = self.tracker.start(filters.clone());
        cx.notify();

        let parsed = match parse_filters(&filters) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.tracker.accept(generation, filters, Err(error));
                return;
            }
        };
        let path = match &self.database_path {
            Ok(path) => path.clone(),
            Err(error) => {
                let error = error.clone();
                self.tracker.accept(generation, filters, Err(error));
                return;
            }
        };
        let task = cx.background_executor().spawn(async move {
            let ParsedFilters {
                from,
                to,
                income_category_id,
                expense_category_id,
            } = parsed;
            finance_db::load_analysis_transactions_by_category(
                &path,
                from,
                to,
                income_category_id,
                expense_category_id,
            )
            .map_err(|error| error.to_string())
            .and_then(|transactions| {
                summarize_analysis(&transactions).map_err(|error| error.to_string())
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |page, cx| {
                if page.tracker.accept(generation, filters, result) {
                    if let Some(summary) = page.tracker.current_report() {
                        page.update_tables(summary, cx);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for date_picker in [&self.from, &self.to] {
            date_picker.update(cx, |state, cx| {
                state.set_date(Date::Single(None), window, cx)
            });
        }
        self.period_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        for picker in [&mut self.income, &mut self.expense] {
            picker.selected = None;
            picker.select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
            });
        }
        self.apply_pending_filters(cx);
    }

    fn clear_date(&mut self, to: bool, window: &mut Window, cx: &mut Context<Self>) {
        let date_picker = if to { &self.to } else { &self.from };
        date_picker.update(cx, |state, cx| {
            state.set_date(Date::Single(None), window, cx)
        });
        self.apply_pending_filters(cx);
    }

    fn clear_category(&mut self, income: bool, window: &mut Window, cx: &mut Context<Self>) {
        let picker = if income {
            &mut self.income
        } else {
            &mut self.expense
        };
        picker.selected = None;
        picker.select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::default().row(0)), window, cx)
        });
        self.apply_pending_filters(cx);
    }

    // ---- отрисовка ----

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

    fn render_filters(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.has_active_filters(cx);
        let from_set = selected_date(self.from.read(cx)).is_some();
        let to_set = selected_date(self.to.read(cx)).is_some();
        let income_set = self.income.selected.is_some();
        let expense_set = self.expense.selected.is_some();
        // Подпись над полем; кнопка «×» справа от подписи, если фильтр задан.
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
                        "Период",
                        140.0,
                        None,
                        Select::new(&self.period_select)
                            .placeholder("Период")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "С",
                        150.0,
                        from_set.then(|| {
                            clear(Self::clear_button(
                                "analysis-clear-from",
                                "Очистить начальную дату",
                                cx,
                                |page, window, cx| page.clear_date(false, window, cx),
                            ))
                        }),
                        DatePicker::new(&self.from)
                            .placeholder("Выбрать дату")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "По",
                        150.0,
                        to_set.then(|| {
                            clear(Self::clear_button(
                                "analysis-clear-to",
                                "Очистить конечную дату",
                                cx,
                                |page, window, cx| page.clear_date(true, window, cx),
                            ))
                        }),
                        DatePicker::new(&self.to)
                            .placeholder("Выбрать дату")
                            .small()
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Категория доходов",
                        260.0,
                        income_set.then(|| {
                            clear(Self::clear_button(
                                "analysis-clear-income",
                                "Очистить категорию доходов",
                                cx,
                                |page, window, cx| page.clear_category(true, window, cx),
                            ))
                        }),
                        div()
                            .id("income-category-select")
                            .child(
                                Select::new(&self.income.select)
                                    .placeholder(NONE_LABEL)
                                    .small(),
                            )
                            .when_some(
                                self.income
                                    .select
                                    .read(cx)
                                    .selected_index(cx)
                                    .map(|ix| ix.row)
                                    .and_then(|row| self.income.labels.get(row).cloned())
                                    .filter(|label| label != NONE_LABEL),
                                |el, tooltip| {
                                    el.tooltip(move |window, cx| {
                                        ComponentTooltip::new(tooltip.clone()).build(window, cx)
                                    })
                                },
                            )
                            .into_any_element(),
                    ))
                    .child(filter(
                        "Категория расходов",
                        260.0,
                        expense_set.then(|| {
                            clear(Self::clear_button(
                                "analysis-clear-expense",
                                "Очистить категорию расходов",
                                cx,
                                |page, window, cx| page.clear_category(false, window, cx),
                            ))
                        }),
                        div()
                            .id("expense-category-select")
                            .child(
                                Select::new(&self.expense.select)
                                    .placeholder(NONE_LABEL)
                                    .small(),
                            )
                            .when_some(
                                self.expense
                                    .select
                                    .read(cx)
                                    .selected_index(cx)
                                    .map(|ix| ix.row)
                                    .and_then(|row| self.expense.labels.get(row).cloned())
                                    .filter(|label| label != NONE_LABEL),
                                |el, tooltip| {
                                    el.tooltip(move |window, cx| {
                                        ComponentTooltip::new(tooltip.clone()).build(window, cx)
                                    })
                                },
                            )
                            .into_any_element(),
                    )),
            )
            .child(div().text_size(px(12.0)).child(
                "Выбранная категория и все её подкатегории учитываются в таблице и диаграмме.",
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("analysis-apply")
                            .primary()
                            .small()
                            .label("Применить")
                            .on_click(cx.listener(|page, _, _, cx| page.apply_pending_filters(cx))),
                    )
                    .child(
                        Button::new("analysis-reset")
                            .small()
                            .label("Сбросить фильтры")
                            .disabled(!active)
                            .on_click(
                                cx.listener(|page, _, window, cx| page.reset_filters(window, cx)),
                            ),
                    ),
            )
    }

    fn summary_row(
        kind: TransactionKind,
        values: &[String],
        strong: bool,
        header: bool,
        stripe: Option<Hsla>,
        border: Hsla,
        row_id: (&'static str, usize),
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .w(px(analysis_table_width(kind)))
            .h(px(ROW_HEIGHT))
            .border_l_1()
            .border_r_1()
            .border_b_1()
            .when(header, |row| row.border_t_1())
            .border_color(border)
            .when(strong, |row| row.font_weight(FontWeight::BOLD))
            .when_some(stripe, |row, color| row.bg(color))
            .children(values.iter().enumerate().map(|(index, source)| {
                let value = source.clone();
                let cell = div()
                    .w(px(COLUMN_WIDTHS[index]))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .when(index > 0, |cell| cell.border_l_1().border_color(border))
                    .px_2()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .when(index > 0, |cell| cell.justify_end().text_right());
                let tooltip = value.clone();
                if index == 0 {
                    cell.id(row_id)
                        .tooltip(move |window, cx| {
                            ComponentTooltip::new(tooltip.clone()).build(window, cx)
                        })
                        .child(value)
                        .into_any_element()
                } else {
                    cell.child(value).into_any_element()
                }
            }))
            .into_any_element()
    }

    fn render_kind_summary(&self, kind: TransactionKind, border: Hsla, cx: &App) -> Div {
        let mut column = div().flex().flex_col().gap_2().w_full().h_full().min_h_0();
        let mut has_totals = false;
        for currency in Currency::ALL {
            let table_index = analysis_table_index(kind, currency);
            let table_state = self.tables[table_index].read(cx);
            let Some(total) = table_state.delegate().total.as_ref() else {
                continue;
            };
            has_totals = true;
            let table = DataTable::new(&self.tables[table_index])
                .stripe(true)
                .bordered(true);
            let total_label = if kind == TransactionKind::Expense {
                "Итого по расходам"
            } else {
                "Итого"
            };
            let total_values = if kind == TransactionKind::Expense {
                vec![
                    "Итого по расходам".to_owned(),
                    total.transaction_count.to_string(),
                    format_currency(&total.amount, 2),
                    "100%".to_owned(),
                    "—".to_owned(),
                ]
            } else {
                vec![
                    total_label.to_owned(),
                    total.transaction_count.to_string(),
                    format_currency(&total.amount, 2),
                    "100%".to_owned(),
                ]
            };
            let mut footer = vec![Self::summary_row(
                kind,
                &total_values,
                true,
                false,
                None,
                border,
                ("analysis-total", table_index),
            )];
            if let Some(values) = regular_summary_values(kind, total) {
                footer.push(Self::summary_row(
                    kind,
                    &values,
                    false,
                    false,
                    None,
                    border,
                    ("analysis-regular-total", table_index),
                ));
            }
            let table_content = div()
                .w(px(analysis_table_width(kind)))
                .h_full()
                .min_h_0()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().w_full().child(table))
                .children(footer);
            column = column.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_lg().font_weight(FontWeight::BOLD).child(format!(
                        "{} — {}",
                        kind.title(),
                        currency.code()
                    )))
                    .child(
                        div()
                            .id(("analysis-table-scroll", table_index))
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .overflow_x_scrollbar()
                            .child(table_content),
                    ),
            );
        }
        if !has_totals {
            column = column.child(format!(
                "За выбранный период операций типа «{}» нет.",
                kind.title()
            ));
        }
        column
    }

    fn render_summary(
        &self,
        summary: &AnalysisSummary,
        border: Hsla,
        cx: &App,
    ) -> impl IntoElement {
        if summary.totals.is_empty() {
            return div()
                .child(selectable_text(
                    "analysis-empty-period",
                    "За выбранный период операций нет.",
                ))
                .into_any_element();
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .gap_4()
            .children(
                [
                    (TransactionKind::Expense, "analysis-expense-scroll"),
                    (TransactionKind::Income, "analysis-income-scroll"),
                ]
                .into_iter()
                .map(|(kind, id)| {
                    div()
                        .id(id)
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(self.render_kind_summary(kind, border, cx))
                }),
            )
            .into_any_element()
    }

    fn render_pie_charts(
        &self,
        summary: &AnalysisSummary,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        const LEGEND_VISIBLE_MAX: usize = 4;
        let is_dark = cx.theme().is_dark();
        div().flex().gap_4().children(
            [TransactionKind::Expense, TransactionKind::Income]
                .into_iter()
                .map(|kind| {
                    let mut column = div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_start()
                        .gap_x_4()
                        .gap_y_2();
                    for currency in Currency::ALL {
                        if !summary
                            .totals
                            .iter()
                            .any(|total| total.kind == kind && total.currency == currency)
                        {
                            continue;
                        }
                        let heading = div().font_weight(FontWeight::BOLD).child(format!(
                            "{} — {}",
                            kind.title(),
                            currency.code()
                        ));
                        let Some(slices) = pie_chart_slices(summary, kind, currency) else {
                            column =
                                column.child(div().flex().flex_col().gap_1().child(heading).child(
                                    selectable_text(
                                        (
                                            "analysis-chart-error",
                                            analysis_table_index(kind, currency),
                                        ),
                                        "Не удалось построить диаграмму для этих данных.",
                                    ),
                                ));
                            continue;
                        };
                        if slices.is_empty() {
                            continue;
                        }
                        let geometry: Vec<(i64, f32)> = slices
                            .iter()
                            .map(|slice| (slice.category.category_id, slice.share))
                            .collect();
                        let visible: Vec<_> =
                            slices[..LEGEND_VISIBLE_MAX.min(slices.len())].to_vec();
                        let hidden: Vec<_> =
                            slices[LEGEND_VISIBLE_MAX.min(slices.len())..].to_vec();
                        let legend_item =
                            |slice: &PieChartSlice| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div().size(px(12.0)).flex_none().rounded_sm().bg(
                                            pie_chart_color(slice.category.category_id, is_dark),
                                        ),
                                    )
                                    .child(format!(
                                        "{} — {}%",
                                        slice.category.category_name,
                                        slice.category.share_percent.round_dp(2)
                                    ))
                            };
                        // Соберём данные скрытых элементов как owned-значения для замыкания on_click.
                        let hidden_data: Vec<(i64, String, f64)> = hidden
                            .iter()
                            .map(|slice| {
                                (
                                    slice.category.category_id,
                                    slice.category.category_name.clone(),
                                    slice.category.share_percent.to_f64().unwrap_or(0.0),
                                )
                            })
                            .collect();
                        let kind_title = kind.title().to_owned();
                        let currency_code = currency.code().to_owned();
                        let legend = div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::BOLD).child("Категории"))
                            .children(visible.iter().map(legend_item))
                            .when(!hidden_data.is_empty(), |legend| {
                                let hidden = hidden_data.clone();
                                let more_label = format!("Ещё {}…", hidden.len());
                                let kind_t = kind_title.clone();
                                let curr_c = currency_code.clone();
                                legend.child(
                                    div()
                                        .id((
                                            "legend-more",
                                            (kind as usize) << 8 | currency as usize,
                                        ))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .cursor_pointer()
                                        .text_color(cx.theme().accent_foreground)
                                        .underline()
                                        .child(more_label)
                                        .on_click(move |_, window, cx| {
                                            let hidden = hidden.clone();
                                            let kind_t = kind_t.clone();
                                            let curr_c = curr_c.clone();
                                            window.open_dialog(cx, move |dialog: Dialog, _, _| {
                                                let items: Vec<_> = hidden
                                                    .iter()
                                                    .map(|(cat_id, name, share)| {
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(
                                                                div()
                                                                    .size(px(12.0))
                                                                    .flex_none()
                                                                    .rounded_sm()
                                                                    .bg(pie_chart_color(
                                                                        *cat_id, is_dark,
                                                                    )),
                                                            )
                                                            .child(format!(
                                                                "{} — {:.2}%",
                                                                name, share
                                                            ))
                                                            .into_any_element()
                                                    })
                                                    .collect();
                                                dialog
                                                    .title(format!(
                                                        "Категории — {}, {}",
                                                        kind_t, curr_c
                                                    ))
                                                    .centered()
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap_1()
                                                            .children(items),
                                                    )
                                            });
                                        }),
                                )
                            });
                        column = column.child(
                            div().flex().flex_col().gap_1().child(heading).child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(pie_chart(geometry, is_dark))
                                    .child(legend),
                            ),
                        );
                    }
                    column
                }),
        )
    }

    fn render_footer(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let danger = cx.theme().danger;
        let notes = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(selectable_text(
                "analysis-shares-note",
                "Доли считаются отдельно для доходов и расходов и только внутри одной валюты. RUB и USD не пересчитываются.",
            ))
            .child(selectable_message(
                "analysis-forecast-note",
                "Прогноз и оценка процентов не рассчитываются: методика и необходимые исходные данные не согласованы.",
                cx.theme().warning,
            ));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .border_t_1()
            .border_color(border)
            .pt_2()
            .when_some(self.tracker.current_report(), |footer, summary| {
                footer
                    .child(render_balance_summary(summary, danger))
                    .child(self.render_pie_charts(summary, window, cx))
            })
            .child(notes)
    }
}

impl Render for AnalysisPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let border = cx.theme().border;
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
                    .child("Анализ"),
            )
            .child(
                Button::new("refresh-analysis")
                    .secondary()
                    .large()
                    .label("Обновить")
                    .icon(IconName::Replace)
                    .on_click(cx.listener(|page, _, window, cx| page.reload(window, cx))),
            );
        root = root.child(header);
        root = root.child(self.render_filters(cx));
        if let Some(message) = &self.message {
            root = root.child(selectable_message(
                "analysis-page-error",
                message.clone(),
                danger,
            ));
        }
        let body = if let Some(error) = self.tracker.current_error() {
            div()
                .flex_1()
                .text_color(danger)
                .child(selectable_text("analysis-load-error", error.to_owned()))
                .into_any_element()
        } else if let Some(summary) = self.tracker.current_report() {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.render_summary(summary, border, cx))
                .into_any_element()
        } else if self.tracker.loading {
            div()
                .flex_1()
                .child("Загружаем операции…")
                .into_any_element()
        } else {
            div().flex_1().into_any_element()
        };
        root.child(body).child(self.render_footer(window, cx))
    }
}

fn category_tree_labels(categories: &[Category]) -> Vec<(i64, String)> {
    super::support::category_tree_labels(categories)
}

fn analysis_balance_lines(
    summary: &AnalysisSummary,
) -> Result<Vec<String>, TransactionBalanceError> {
    let transactions = summary
        .totals
        .iter()
        .map(|total| TransactionAmount {
            kind: total.kind,
            amount: total.amount,
            currency: total.currency,
        })
        .collect::<Vec<_>>();
    let balances = summarize_transaction_balance(&transactions)?;

    Ok(balances
        .into_iter()
        .filter(|balance| {
            summary
                .totals
                .iter()
                .any(|total| total.currency == balance.currency)
        })
        .map(|balance| {
            format!(
                "{} — Доходы: {} {}; Расходы: {} {}; Сальдо: {} {}",
                balance.currency.code(),
                format_currency(&balance.income, 2),
                balance.currency.code(),
                format_currency(&balance.expenses, 2),
                balance.currency.code(),
                format_currency(&balance.balance, 2),
                balance.currency.code()
            )
        })
        .collect())
}

fn render_balance_summary(summary: &AnalysisSummary, danger: Hsla) -> Div {
    match analysis_balance_lines(summary) {
        Ok(lines) => div()
            .flex()
            .flex_col()
            .gap_1()
            .children(lines.into_iter().enumerate().map(|(index, line)| {
                div().child(selectable_text(("analysis-balance-line", index), line))
            })),
        Err(error) => selectable_message(
            "analysis-balance-error",
            format!("Не удалось рассчитать сальдо: {error}"),
            danger,
        ),
    }
}

#[cfg(test)]
mod tests {
    // Глоб-импорт `gpui_kit::*` затеняет атрибут `#[test]`, поэтому имена импортируются явно.
    use super::{
        analysis_balance_lines, analysis_column_headers, analysis_table_width, parse_filters,
        pie_chart_color, pie_chart_slices, pie_slice_points, regular_label, regular_summary_values,
        sort_analysis_categories, AnalysisFilters, AnalysisTable, CategoryFilterItem,
        CategoryPicker, RequestTracker, COL_AMOUNT, COL_CATEGORY, COL_REGULAR, COL_SHARE,
        COL_TRANSACTION_COUNT, EXPENSE_CATEGORY_FILTER_ITEM_ID, EXPENSE_COLUMN_HEADERS,
        INCOME_CATEGORY_FILTER_ITEM_ID, INCOME_COLUMN_HEADERS, PIE_CHART_COLORS_DARK,
        PIE_CHART_COLORS_LIGHT,
    };
    use chrono::NaiveDate;
    use finance_core::{
        summarize_analysis, AnalysisCategoryTotal, AnalysisRegularTotal, AnalysisSummary,
        AnalysisTotal, AnalysisTransaction, Category, Currency, TransactionBalanceError,
        TransactionKind,
    };
    use gpui_kit::component::select::SelectItem as _;
    use rust_decimal::Decimal;

    fn summary_with_both_kinds_and_currencies() -> AnalysisSummary {
        let categories = [
            (TransactionKind::Income, Currency::Rub, "Зарплата"),
            (TransactionKind::Income, Currency::Rub, "Премия"),
            (TransactionKind::Income, Currency::Usd, "Доход USD"),
            (TransactionKind::Income, Currency::Usd, "Бонус USD"),
            (TransactionKind::Expense, Currency::Rub, "Покупки"),
            (
                TransactionKind::Expense,
                Currency::Rub,
                "Коммунальные услуги",
            ),
            (TransactionKind::Expense, Currency::Usd, "Расход USD"),
            (TransactionKind::Expense, Currency::Usd, "Платеж USD"),
        ]
        .into_iter()
        .map(|(kind, currency, category_name)| AnalysisCategoryTotal {
            kind,
            category_id: 1,
            category_name: category_name.to_owned(),
            currency,
            amount: Decimal::ONE,
            transaction_count: 1,
            share_percent: Decimal::new(50, 0),
            is_regular: false,
        })
        .collect();
        let totals = [
            (TransactionKind::Income, Currency::Rub),
            (TransactionKind::Income, Currency::Usd),
            (TransactionKind::Expense, Currency::Rub),
            (TransactionKind::Expense, Currency::Usd),
        ]
        .into_iter()
        .map(|(kind, currency)| AnalysisTotal {
            kind,
            currency,
            amount: Decimal::new(2, 0),
            transaction_count: 2,
            regular: None,
        })
        .collect();
        AnalysisSummary { categories, totals }
    }

    fn category_total(
        id: i64,
        name: &str,
        amount: Decimal,
        transaction_count: usize,
        share_percent: Decimal,
        is_regular: bool,
    ) -> AnalysisCategoryTotal {
        AnalysisCategoryTotal {
            kind: TransactionKind::Income,
            category_id: id,
            category_name: name.to_owned(),
            currency: Currency::Rub,
            amount,
            transaction_count,
            share_percent,
            is_regular,
        }
    }

    fn sortable_categories() -> Vec<AnalysisCategoryTotal> {
        vec![
            category_total(
                1,
                "Бета",
                Decimal::new(95, 1),
                2,
                Decimal::new(1225, 2),
                true,
            ),
            category_total(2, "Альфа", Decimal::from(100), 10, Decimal::from(50), false),
            category_total(
                3,
                "Гамма",
                Decimal::from(20),
                5,
                Decimal::new(3775, 2),
                false,
            ),
        ]
    }

    fn summary_with_categories(
        categories: Vec<AnalysisCategoryTotal>,
        total_amount: Decimal,
        transaction_count: usize,
    ) -> AnalysisSummary {
        AnalysisSummary {
            categories,
            totals: vec![AnalysisTotal {
                kind: TransactionKind::Income,
                currency: Currency::Rub,
                amount: total_amount,
                transaction_count,
                regular: None,
            }],
        }
    }

    fn filters(from: &str, to: &str, income: Option<i64>, expense: Option<i64>) -> AnalysisFilters {
        AnalysisFilters {
            from: (!from.is_empty()).then(|| {
                NaiveDate::parse_from_str(from, super::DATE_FORMAT)
                    .expect("test date should use the supported format")
            }),
            to: (!to.is_empty()).then(|| {
                NaiveDate::parse_from_str(to, super::DATE_FORMAT)
                    .expect("test date should use the supported format")
            }),
            income_category_id: income,
            expense_category_id: expense,
        }
    }

    fn empty_summary() -> AnalysisSummary {
        AnalysisSummary {
            categories: Vec::new(),
            totals: Vec::new(),
        }
    }

    #[test]
    fn analysis_balance_lines_show_each_currency_and_negative_balance_separately() {
        let summary = AnalysisSummary {
            categories: Vec::new(),
            totals: vec![
                AnalysisTotal {
                    kind: TransactionKind::Income,
                    currency: Currency::Rub,
                    amount: Decimal::new(10_050, 2),
                    transaction_count: 1,
                    regular: None,
                },
                AnalysisTotal {
                    kind: TransactionKind::Expense,
                    currency: Currency::Rub,
                    amount: Decimal::from(125),
                    transaction_count: 1,
                    regular: None,
                },
                AnalysisTotal {
                    kind: TransactionKind::Income,
                    currency: Currency::Usd,
                    amount: Decimal::from(35),
                    transaction_count: 1,
                    regular: None,
                },
                AnalysisTotal {
                    kind: TransactionKind::Expense,
                    currency: Currency::Usd,
                    amount: Decimal::new(1_225, 2),
                    transaction_count: 1,
                    regular: None,
                },
            ],
        };

        assert_eq!(
            analysis_balance_lines(&summary),
            Ok(vec![
                "RUB — Доходы: 100.50 RUB; Расходы: 125.00 RUB; Сальдо: -24.50 RUB".to_owned(),
                "USD — Доходы: 35.00 USD; Расходы: 12.25 USD; Сальдо: 22.75 USD".to_owned(),
            ])
        );
    }

    #[test]
    fn analysis_balance_lines_use_zero_for_missing_transaction_kind() {
        let summary = AnalysisSummary {
            categories: Vec::new(),
            totals: vec![
                AnalysisTotal {
                    kind: TransactionKind::Income,
                    currency: Currency::Rub,
                    amount: Decimal::from(17),
                    transaction_count: 1,
                    regular: None,
                },
                AnalysisTotal {
                    kind: TransactionKind::Expense,
                    currency: Currency::Usd,
                    amount: Decimal::from(5),
                    transaction_count: 1,
                    regular: None,
                },
            ],
        };

        assert_eq!(
            analysis_balance_lines(&summary),
            Ok(vec![
                "RUB — Доходы: 17.00 RUB; Расходы: 0.00 RUB; Сальдо: 17.00 RUB".to_owned(),
                "USD — Доходы: 0.00 USD; Расходы: 5.00 USD; Сальдо: -5.00 USD".to_owned(),
            ])
        );
    }

    #[test]
    fn analysis_balance_lines_are_empty_when_report_has_no_transactions() {
        assert_eq!(analysis_balance_lines(&empty_summary()), Ok(Vec::new()));
    }

    #[test]
    fn analysis_balance_lines_return_calculation_errors() {
        let summary = AnalysisSummary {
            categories: Vec::new(),
            totals: vec![
                AnalysisTotal {
                    kind: TransactionKind::Income,
                    currency: Currency::Rub,
                    amount: Decimal::MAX,
                    transaction_count: 1,
                    regular: None,
                },
                AnalysisTotal {
                    kind: TransactionKind::Income,
                    currency: Currency::Rub,
                    amount: Decimal::ONE,
                    transaction_count: 1,
                    regular: None,
                },
            ],
        };

        assert_eq!(
            analysis_balance_lines(&summary),
            Err(TransactionBalanceError::Overflow)
        );
    }

    #[test]
    fn pie_chart_slices_are_separated_by_operation_kind_and_currency() {
        let summary = summary_with_both_kinds_and_currencies();

        let income_rub = pie_chart_slices(&summary, TransactionKind::Income, Currency::Rub)
            .expect("income shares should be representable for chart geometry");
        let expense_usd = pie_chart_slices(&summary, TransactionKind::Expense, Currency::Usd)
            .expect("expense shares should be representable for chart geometry");

        assert_eq!(
            income_rub
                .iter()
                .map(|slice| slice.category.category_name.as_str())
                .collect::<Vec<_>>(),
            ["Зарплата", "Премия"]
        );
        assert_eq!(
            expense_usd
                .iter()
                .map(|slice| slice.category.category_name.as_str())
                .collect::<Vec<_>>(),
            ["Расход USD", "Платеж USD"]
        );
    }

    #[test]
    fn pie_chart_includes_each_category_total_in_the_selected_branch() {
        let transactions = [
            (1, "Зарплата", 10),
            (2, "Премии", 20),
            (3, "Годовая премия", 30),
        ]
        .into_iter()
        .map(|(category_id, category_name, amount)| AnalysisTransaction {
            kind: TransactionKind::Income,
            category_id,
            category_name: category_name.to_owned(),
            amount: Decimal::from(amount),
            currency: Currency::Rub,
            is_regular: false,
        })
        .collect::<Vec<_>>();
        let summary = summarize_analysis(&transactions).unwrap();

        let slices = pie_chart_slices(&summary, TransactionKind::Income, Currency::Rub)
            .expect("category totals should be representable as chart slices");

        assert_eq!(
            slices
                .iter()
                .map(|slice| slice.category.category_name.as_str())
                .collect::<Vec<_>>(),
            ["Зарплата", "Премии", "Годовая премия"]
        );
    }

    #[test]
    fn empty_analysis_summary_has_no_pie_chart() {
        let summary = empty_summary();
        let slices = pie_chart_slices(&summary, TransactionKind::Income, Currency::Rub)
            .expect("empty summary is representable");
        assert!(slices.is_empty());
    }

    #[test]
    fn analysis_table_sorts_each_column_in_both_directions() {
        let cases = [
            (COL_CATEGORY, [2, 1, 3], [3, 1, 2]),
            (COL_TRANSACTION_COUNT, [1, 3, 2], [2, 3, 1]),
            (COL_AMOUNT, [1, 3, 2], [2, 3, 1]),
            (COL_SHARE, [1, 3, 2], [2, 3, 1]),
            (COL_REGULAR, [2, 3, 1], [1, 2, 3]),
        ];

        for (column, ascending, descending) in cases {
            let mut rows = sortable_categories();
            sort_analysis_categories(&mut rows, Some((column, super::ColumnSort::Ascending)));
            assert_eq!(
                rows.iter().map(|row| row.category_id).collect::<Vec<_>>(),
                ascending
            );

            sort_analysis_categories(&mut rows, Some((column, super::ColumnSort::Descending)));
            assert_eq!(
                rows.iter().map(|row| row.category_id).collect::<Vec<_>>(),
                descending
            );
        }
    }

    #[test]
    fn analysis_table_keeps_group_totals_outside_category_rows() {
        let summary = summary_with_both_kinds_and_currencies();
        let mut income_rub_table = AnalysisTable::new(TransactionKind::Income, Currency::Rub);
        let mut expense_usd_table = AnalysisTable::new(TransactionKind::Expense, Currency::Usd);

        income_rub_table.set_summary(&summary);
        expense_usd_table.set_summary(&summary);

        assert_eq!(
            income_rub_table
                .rows
                .iter()
                .map(|row| row.category_name.as_str())
                .collect::<Vec<_>>(),
            ["Зарплата", "Премия"]
        );
        assert_eq!(
            expense_usd_table
                .rows
                .iter()
                .map(|row| row.category_name.as_str())
                .collect::<Vec<_>>(),
            ["Платеж USD", "Расход USD"]
        );
        assert_eq!(
            income_rub_table.total.as_ref().map(|total| total.amount),
            Some(Decimal::from(2))
        );
        assert_eq!(
            expense_usd_table
                .total
                .as_ref()
                .map(|total| total.transaction_count),
            Some(2)
        );
        assert_eq!(income_rub_table.rows.len(), 2);
        assert_eq!(expense_usd_table.rows.len(), 2);
    }

    #[test]
    fn analysis_table_refresh_replaces_rows_and_preserves_sort_order() {
        let mut table = AnalysisTable::new(TransactionKind::Income, Currency::Rub);
        table.sort = Some((COL_AMOUNT, super::ColumnSort::Ascending));
        table.set_summary(&summary_with_categories(
            sortable_categories(),
            Decimal::new(1295, 1),
            17,
        ));
        let refreshed = summary_with_categories(
            vec![
                category_total(
                    4,
                    "Новая Бета",
                    Decimal::from(10),
                    1,
                    Decimal::from(25),
                    false,
                ),
                category_total(
                    5,
                    "Новая Альфа",
                    Decimal::from(3),
                    2,
                    Decimal::from(75),
                    false,
                ),
            ],
            Decimal::from(13),
            3,
        );

        table.set_summary(&refreshed);

        assert_eq!(
            table
                .rows
                .iter()
                .map(|row| row.category_id)
                .collect::<Vec<_>>(),
            [5, 4]
        );
        assert_eq!(
            table.total.as_ref().map(|total| total.amount),
            Some(Decimal::from(13))
        );
    }

    #[test]
    fn pie_slice_points_start_at_center_and_follow_the_arc() {
        let points = pie_slice_points((10.0, 10.0), 5.0, 0.0, std::f32::consts::FRAC_PI_2);
        assert_eq!(points[0], (10.0, 10.0));
        let first = points[1];
        let last = points[points.len() - 1];
        assert!((first.0 - 15.0).abs() < 1e-4 && (first.1 - 10.0).abs() < 1e-4);
        assert!((last.0 - 10.0).abs() < 1e-4 && (last.1 - 15.0).abs() < 1e-4);
    }

    #[test]
    fn pie_chart_uses_the_configured_palette_for_each_theme() {
        assert_eq!(
            PIE_CHART_COLORS_DARK,
            [0x4e7db1, 0xcd6e5c, 0x379291, 0xcfb171, 0x7b5da0, 0xd79669, 0x61855b, 0xad5d76,]
        );
        assert_eq!(
            PIE_CHART_COLORS_LIGHT,
            [0x3975a8, 0xbd523c, 0x247e75, 0xa47b22, 0x78539b, 0xbe692c, 0x5a7d3c, 0xaa4c67,]
        );
    }

    #[test]
    fn pie_chart_colors_repeat_for_categories_outside_each_palette() {
        for is_dark in [false, true] {
            for category_id in -8..8 {
                assert_eq!(
                    pie_chart_color(category_id, is_dark),
                    pie_chart_color(category_id + 8, is_dark)
                );
            }
        }
    }

    #[test]
    fn stale_analysis_result_does_not_replace_current_filter_result() {
        let mut tracker = RequestTracker::default();
        let previous = filters("", "", None, None);
        let previous_generation = tracker.start(previous.clone());
        let current = filters("01.01.2027", "", Some(2), Some(1));
        tracker.start(current);

        let accepted = tracker.accept(previous_generation, previous, Ok(empty_summary()));

        assert!(!accepted);
        assert!(tracker.report.is_none());
        assert!(tracker.loading);
    }

    #[test]
    fn current_result_is_accepted_and_failed_reports_are_hidden() {
        let mut tracker = RequestTracker::default();
        let applied = filters("", "", None, None);
        let generation = tracker.start(applied.clone());
        assert!(tracker.accept(generation, applied.clone(), Ok(empty_summary())));
        assert!(tracker.current_report().is_some());

        let generation = tracker.start(applied.clone());
        assert!(tracker.accept(generation, applied, Err("сбой".to_owned())));
        assert_eq!(tracker.current_error(), Some("сбой"));
        assert!(tracker.current_report().is_none());
    }

    #[test]
    fn report_for_other_filters_is_not_current() {
        let mut tracker = RequestTracker::default();
        let first = filters("", "", None, None);
        let generation = tracker.start(first.clone());
        tracker.accept(generation, first, Ok(empty_summary()));

        tracker.start(filters("01.01.2027", "", None, None));

        assert!(tracker.current_report().is_none());
    }

    #[test]
    fn analysis_filter_rejects_reversed_period_boundaries() {
        assert_eq!(
            parse_filters(&filters("01.02.2027", "31.01.2027", None, None)),
            Err("Начало периода не может быть позже его окончания".to_owned())
        );
    }

    #[test]
    fn analysis_filter_parses_dates_and_categories() {
        let parsed = parse_filters(&filters("01.01.2027", "", Some(3), None))
            .expect("valid filters should parse");
        assert_eq!(parsed.from, NaiveDate::from_ymd_opt(2027, 1, 1));
        assert_eq!(parsed.to, None);
        assert_eq!(parsed.income_category_id, Some(3));
        assert_eq!(parsed.expense_category_id, None);
    }

    #[test]
    fn category_picker_maps_confirmed_label_to_category_id() {
        let categories = vec![
            Category {
                id: 7,
                name: "Зарплата".to_owned(),
                parent_id: None,
                is_active: true,
                is_regular: None,
            },
            Category {
                id: 9,
                name: "Премия".to_owned(),
                parent_id: Some(7),
                is_active: true,
                is_regular: None,
            },
        ];
        let (ids, labels) = CategoryPicker::labels_for(&categories);
        assert_eq!(ids, [7, 9]);
        assert_eq!(labels, ["Все", "Зарплата", "Зарплата > Премия"]);
    }

    #[test]
    fn analysis_category_filter_items_keep_full_labels_for_tooltips() {
        let income = CategoryFilterItem {
            id: 1,
            label: "Зарплата > Премия".to_owned(),
            item_id: INCOME_CATEGORY_FILTER_ITEM_ID,
        };
        let expense = CategoryFilterItem {
            id: 2,
            label: "Транспорт > Разное > История".to_owned(),
            item_id: EXPENSE_CATEGORY_FILTER_ITEM_ID,
        };

        assert_eq!(income.title().to_string(), "Зарплата > Премия");
        assert_eq!(income.value(), "Зарплата > Премия");
        assert_eq!(expense.title().to_string(), "Транспорт > Разное > История");
        assert_eq!(expense.value(), "Транспорт > Разное > История");
        assert_ne!(income.item_id, expense.item_id);
    }
    #[test]
    fn only_expense_analysis_tables_have_the_regular_column() {
        assert_eq!(
            analysis_column_headers(TransactionKind::Expense),
            EXPENSE_COLUMN_HEADERS
        );
        assert_eq!(
            analysis_column_headers(TransactionKind::Income),
            INCOME_COLUMN_HEADERS
        );
        assert_eq!(analysis_table_width(TransactionKind::Expense), 600.0);
        assert_eq!(analysis_table_width(TransactionKind::Income), 505.0);
    }

    #[test]
    fn regular_column_uses_explicit_yes_and_no_labels() {
        assert_eq!(regular_label(true), "Да");
        assert_eq!(regular_label(false), "Нет");
    }

    #[test]
    fn regular_footer_row_is_only_shown_for_expenses() {
        let income_total = AnalysisTotal {
            kind: TransactionKind::Income,
            currency: Currency::Rub,
            amount: Decimal::from(100),
            transaction_count: 2,
            regular: Some(AnalysisRegularTotal {
                amount: Decimal::from(50),
                transaction_count: 1,
                share_percent: Decimal::from(50),
            }),
        };
        assert!(regular_summary_values(TransactionKind::Income, &income_total).is_none());

        let zero_total = AnalysisTotal {
            kind: TransactionKind::Expense,
            currency: Currency::Rub,
            amount: Decimal::from(5),
            transaction_count: 1,
            regular: Some(AnalysisRegularTotal {
                amount: Decimal::ZERO,
                transaction_count: 0,
                share_percent: Decimal::ZERO,
            }),
        };
        assert_eq!(
            regular_summary_values(TransactionKind::Expense, &zero_total),
            Some([
                "Итого в регулярных расходах".to_owned(),
                "0".to_owned(),
                "0.00".to_owned(),
                "0%".to_owned(),
                "—".to_owned(),
            ])
        );

        let positive_total = AnalysisTotal {
            kind: TransactionKind::Expense,
            currency: Currency::Usd,
            amount: Decimal::from(200),
            transaction_count: 4,
            regular: Some(AnalysisRegularTotal {
                amount: Decimal::from(50),
                transaction_count: 2,
                share_percent: Decimal::from(25),
            }),
        };
        assert_eq!(
            regular_summary_values(TransactionKind::Expense, &positive_total),
            Some([
                "Итого в регулярных расходах".to_owned(),
                "2".to_owned(),
                "50.00".to_owned(),
                "25%".to_owned(),
                "—".to_owned(),
            ])
        );
    }

    #[test]
    fn analysis_table_extracts_regular_total_only_for_expenses() {
        let mut summary = summary_with_both_kinds_and_currencies();
        for total in &mut summary.totals {
            if total.kind == TransactionKind::Expense {
                total.regular = Some(AnalysisRegularTotal {
                    amount: Decimal::from(7),
                    transaction_count: 3,
                    share_percent: Decimal::from(25),
                });
            }
        }
        let mut income_table = AnalysisTable::new(TransactionKind::Income, Currency::Rub);
        let mut expense_table = AnalysisTable::new(TransactionKind::Expense, Currency::Rub);
        income_table.set_summary(&summary);
        expense_table.set_summary(&summary);

        assert!(income_table.regular_total.is_none());
        assert_eq!(
            expense_table
                .regular_total
                .as_ref()
                .map(|regular| (regular.amount, regular.transaction_count)),
            Some((Decimal::from(7), 3))
        );
    }

    #[test]
    fn regular_categories_do_not_add_extra_pie_chart_slices() {
        let transactions = [(1, "Аренда", 30, true), (2, "Питание", 70, false)]
            .into_iter()
            .map(
                |(category_id, category_name, amount, is_regular)| AnalysisTransaction {
                    kind: TransactionKind::Expense,
                    category_id,
                    category_name: category_name.to_owned(),
                    amount: Decimal::from(amount),
                    currency: Currency::Rub,
                    is_regular,
                },
            )
            .collect::<Vec<_>>();
        let summary = summarize_analysis(&transactions).unwrap();

        let slices = pie_chart_slices(&summary, TransactionKind::Expense, Currency::Rub)
            .expect("category totals should be representable as chart slices");
        assert_eq!(
            slices
                .iter()
                .map(|slice| slice.category.category_name.as_str())
                .collect::<Vec<_>>(),
            ["Аренда", "Питание"]
        );
    }
}
