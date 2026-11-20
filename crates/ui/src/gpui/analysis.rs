//! Страница «Анализ» на `gpui-kit`. Поведение соответствует `analysis_page.rs`:
//! фильтры применяются явно, отчёт читается в фоне отдельным соединением, а
//! устаревшие результаты отбрасываются счётчиком поколений. Расчёты выполняют
//! `finance-core` и `finance-db`; диаграммы рисуются средствами `gpui`.

use std::path::PathBuf;
use std::rc::Rc;

use chrono::{Local, NaiveDate};
use finance_core::{
    summarize_analysis, AnalysisCategoryTotal, AnalysisSummary, Category, Currency, TransactionKind,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IndexPath};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_decimal::prelude::ToPrimitive;

const DATE_FORMAT: &str = "%Y-%m-%d";
const NONE_LABEL: &str = "Все";
const ROW_HEIGHT: f32 = 30.0;
const PIE_CHART_DIAMETER: f32 = 160.0;
const CHARTS_MAX_HEIGHT: f32 = 190.0;
/// Относительные ширины столбцов: категория, операций, сумма, доля.
const COLUMN_WEIGHTS: [f32; 4] = [2.5, 0.8, 1.5, 0.7];
const COLUMN_HEADERS: [&str; 4] = ["Категория", "Операций", "Сумма", "Доля"];
const PIE_CHART_COLORS: [u32; 8] = [
    0x4682b4, 0xe76f51, 0x2a9d8f, 0xe9c46a, 0x805aa0, 0xf4a261, 0x5e8c50, 0xbe5a70,
];

type Choice = Entity<SelectState<Vec<SharedString>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalysisFilters {
    from: String,
    to: String,
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
    let from = parse_optional_date(&filters.from, "начала")?;
    let to = parse_optional_date(&filters.to, "окончания")?;
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

fn parse_optional_date(value: &str, field_name: &str) -> Result<Option<NaiveDate>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(value, DATE_FORMAT)
        .map(Some)
        .map_err(|_| format!("Введите дату {field_name} периода в формате ГГГГ-ММ-ДД"))
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

struct PieChartSlice<'a> {
    category: &'a AnalysisCategoryTotal,
    share: f32,
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

fn pie_chart_color(category_id: i64) -> Hsla {
    let index = category_id.rem_euclid(PIE_CHART_COLORS.len() as i64) as usize;
    Hsla::from(rgb(PIE_CHART_COLORS[index]))
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

fn pie_chart(slices: Vec<(i64, f32)>) -> impl IntoElement {
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
                    window.paint_path(path, pie_chart_color(*category_id));
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
    select: Choice,
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

    fn handle_confirm(&mut self, value: &Option<SharedString>) {
        self.selected = value
            .as_ref()
            .and_then(|label| self.labels.iter().position(|l| l == label.as_ref()))
            .and_then(|position| position.checked_sub(1))
            .and_then(|position| self.ids.get(position).copied());
    }
}

pub(crate) struct AnalysisPage {
    database: Rc<Database>,
    database_path: Result<PathBuf, String>,
    from: Entity<InputState>,
    to: Entity<InputState>,
    period_select: Choice,
    income: CategoryPicker,
    expense: CategoryPicker,
    tracker: RequestTracker,
    message: Option<String>,
    _subscriptions: Vec<Subscription>,
}

fn choice(items: Vec<String>, window: &mut Window, cx: &mut Context<AnalysisPage>) -> Choice {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(0)), window, cx))
}

impl AnalysisPage {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let database_path = match database.path() {
            Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
            _ => Err("Для анализа требуется путь к локальной базе данных.".to_owned()),
        };
        let from = cx.new(|cx| InputState::new(window, cx).placeholder("С: ГГГГ-ММ-ДД"));
        let to = cx.new(|cx| InputState::new(window, cx).placeholder("По: ГГГГ-ММ-ДД"));
        let period_labels = super::support::DatePeriod::ALL
            .into_iter()
            .map(|(_, name)| name.to_owned())
            .collect();
        let period_select = choice(period_labels, window, cx);
        let income_select = choice(vec![NONE_LABEL.to_owned()], window, cx);
        let expense_select = choice(vec![NONE_LABEL.to_owned()], window, cx);

        let mut subscriptions = Vec::new();
        for input in [&from, &to] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.apply_pending_filters(cx);
                    }
                },
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
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.income.handle_confirm(value);
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &expense_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                this.expense.handle_confirm(value);
                cx.notify();
            },
        ));

        let picker = |select: Choice| CategoryPicker {
            categories: Vec::new(),
            ids: Vec::new(),
            labels: vec![NONE_LABEL.to_owned()],
            selected: None,
            select,
        };
        let mut page = Self {
            database,
            database_path,
            from,
            to,
            period_select,
            income: picker(income_select),
            expense: picker(expense_select),
            tracker: RequestTracker::default(),
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
                    let items: Vec<SharedString> =
                        labels.into_iter().map(SharedString::from).collect();
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
            from: self.from.read(cx).value().trim().to_owned(),
            to: self.to.read(cx).value().trim().to_owned(),
            income_category_id: self.income.selected,
            expense_category_id: self.expense.selected,
        }
    }

    fn has_active_filters(&self, cx: &App) -> bool {
        let filters = self.filters(cx);
        !filters.from.is_empty()
            || !filters.to.is_empty()
            || filters.income_category_id.is_some()
            || filters.expense_category_id.is_some()
    }

    fn set_period(
        &mut self,
        period: super::support::DatePeriod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut from = self.from.read(cx).value().to_string();
        let mut to = self.to.read(cx).value().to_string();
        if !super::support::set_period_dates(period, Local::now().date_naive(), &mut from, &mut to)
        {
            return;
        }
        self.from
            .update(cx, |input, cx| input.set_value(from, window, cx));
        self.to
            .update(cx, |input, cx| input.set_value(to, window, cx));
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
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.from, &self.to] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
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
        let input = if to { &self.to } else { &self.from };
        input.update(cx, |input, cx| input.set_value("", window, cx));
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
        enabled: bool,
        tooltip: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        Button::new(id)
            .compact()
            .label("×")
            .tooltip(tooltip)
            .disabled(!enabled)
            .on_click(cx.listener(move |page, _, window, cx| on_click(page, window, cx)))
    }

    fn render_filters(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.has_active_filters(cx);
        let from_set = !self.from.read(cx).value().trim().is_empty();
        let to_set = !self.to.read(cx).value().trim().is_empty();
        let income_set = self.income.selected.is_some();
        let expense_set = self.expense.selected.is_some();
        let labeled = |text: &'static str| div().child(text);
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(labeled("Период"))
            .child(
                div()
                    .w(px(140.0))
                    .child(Select::new(&self.period_select).placeholder("Выбрать")),
            )
            .child(labeled("Период с"))
            .child(div().w(px(140.0)).child(Input::new(&self.from)))
            .child(Self::clear_button(
                "analysis-clear-from",
                from_set,
                "Очистить начальную дату",
                cx,
                |page, window, cx| page.clear_date(false, window, cx),
            ))
            .child(labeled("по"))
            .child(div().w(px(140.0)).child(Input::new(&self.to)))
            .child(Self::clear_button(
                "analysis-clear-to",
                to_set,
                "Очистить конечную дату",
                cx,
                |page, window, cx| page.clear_date(true, window, cx),
            ))
            .child(labeled("Категория доходов:"))
            .child(
                div()
                    .w(px(220.0))
                    .child(Select::new(&self.income.select).placeholder(NONE_LABEL)),
            )
            .child(Self::clear_button(
                "analysis-clear-income",
                income_set,
                "Очистить категорию доходов",
                cx,
                |page, window, cx| page.clear_category(true, window, cx),
            ))
            .child(labeled("Категория расходов:"))
            .child(
                div()
                    .w(px(220.0))
                    .child(Select::new(&self.expense.select).placeholder(NONE_LABEL)),
            )
            .child(Self::clear_button(
                "analysis-clear-expense",
                expense_set,
                "Очистить категорию расходов",
                cx,
                |page, window, cx| page.clear_category(false, window, cx),
            ))
            .child(
                Button::new("analysis-apply")
                    .primary()
                    .label("Применить")
                    .on_click(cx.listener(|page, _, _, cx| page.apply_pending_filters(cx))),
            )
            .child(
                Button::new("analysis-reset")
                    .label("Сбросить фильтры")
                    .disabled(!active)
                    .on_click(cx.listener(|page, _, window, cx| page.reset_filters(window, cx))),
            )
    }

    fn summary_row(
        values: [String; 4],
        strong: bool,
        stripe: Option<Hsla>,
        border: Hsla,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .h(px(ROW_HEIGHT))
            .gap_2()
            .when(strong, |row| {
                row.font_weight(FontWeight::BOLD)
                    .border_b_1()
                    .border_color(border)
            })
            .when_some(stripe, |row, color| row.bg(color))
            .children(values.into_iter().enumerate().map(|(index, value)| {
                let total: f32 = COLUMN_WEIGHTS.iter().sum();
                div()
                    .w(relative(COLUMN_WEIGHTS[index] / total))
                    .flex_none()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .when(index > 0, |cell| cell.text_right())
                    .child(value)
            }))
    }

    fn render_kind_summary(
        summary: &AnalysisSummary,
        kind: TransactionKind,
        stripe: Hsla,
        border: Hsla,
    ) -> Div {
        let mut column = div().flex().flex_col().gap_1().w_full();
        let mut has_totals = false;
        for currency in Currency::ALL {
            let Some(total) = summary
                .totals
                .iter()
                .find(|total| total.kind == kind && total.currency == currency)
            else {
                continue;
            };
            has_totals = true;
            column = column
                .child(div().text_lg().font_weight(FontWeight::BOLD).child(format!(
                    "{} — {}",
                    kind.title(),
                    currency.code()
                )))
                .child(Self::summary_row(
                    COLUMN_HEADERS.map(str::to_owned),
                    true,
                    None,
                    border,
                ));
            for (row_index, category) in summary
                .categories
                .iter()
                .filter(|category| category.kind == kind && category.currency == currency)
                .enumerate()
            {
                column = column.child(Self::summary_row(
                    [
                        category.category_name.clone(),
                        category.transaction_count.to_string(),
                        category.amount.to_string(),
                        format!("{}%", category.share_percent.round_dp(2)),
                    ],
                    false,
                    (row_index % 2 == 1).then_some(stripe),
                    border,
                ));
            }
            column = column
                .child(Self::summary_row(
                    [
                        "Итого".to_owned(),
                        total.transaction_count.to_string(),
                        total.amount.to_string(),
                        "100%".to_owned(),
                    ],
                    true,
                    None,
                    border,
                ))
                .child(div().h(px(12.0)));
        }
        if !has_totals {
            column = column.child(format!(
                "За выбранный период операций типа «{}» нет.",
                kind.title()
            ));
        }
        column
    }

    fn render_summary(summary: &AnalysisSummary, stripe: Hsla, border: Hsla) -> impl IntoElement {
        if summary.totals.is_empty() {
            return div()
                .child("За выбранный период операций нет.")
                .into_any_element();
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .gap_4()
            .children(
                [
                    (TransactionKind::Income, "analysis-income-scroll"),
                    (TransactionKind::Expense, "analysis-expense-scroll"),
                ]
                .into_iter()
                .map(|(kind, id)| {
                    div()
                        .id(id)
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .child(Self::render_kind_summary(summary, kind, stripe, border))
                }),
            )
            .into_any_element()
    }

    fn render_pie_charts(summary: &AnalysisSummary) -> impl IntoElement {
        div().flex().gap_4().children(
            [TransactionKind::Income, TransactionKind::Expense]
                .into_iter()
                .map(|kind| {
                    let mut column = div().flex_1().min_w_0().flex().flex_col().gap_3();
                    for currency in Currency::ALL {
                        if !summary
                            .totals
                            .iter()
                            .any(|total| total.kind == kind && total.currency == currency)
                        {
                            continue;
                        }
                        let heading = div().text_lg().font_weight(FontWeight::BOLD).child(format!(
                            "{} — {}",
                            kind.title(),
                            currency.code()
                        ));
                        let Some(slices) = pie_chart_slices(summary, kind, currency) else {
                            column = column
                                .child(heading)
                                .child("Не удалось построить диаграмму для этих данных.");
                            continue;
                        };
                        if slices.is_empty() {
                            continue;
                        }
                        let geometry: Vec<(i64, f32)> = slices
                            .iter()
                            .map(|slice| (slice.category.category_id, slice.share))
                            .collect();
                        let legend = div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::BOLD).child("Категории"))
                            .children(slices.iter().map(|slice| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(12.0))
                                            .flex_none()
                                            .rounded_sm()
                                            .bg(pie_chart_color(slice.category.category_id)),
                                    )
                                    .child(format!(
                                        "{} — {}%",
                                        slice.category.category_name,
                                        slice.category.share_percent.round_dp(2)
                                    ))
                            }));
                        column = column.child(heading).child(
                            div()
                                .flex()
                                .gap_4()
                                .child(pie_chart(geometry))
                                .child(legend),
                        );
                    }
                    column
                }),
        )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let notes = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                "Доли считаются отдельно для доходов и расходов и только внутри одной валюты. RUB и USD не пересчитываются.",
            )
            .child(div().text_color(cx.theme().warning).child(
                "Прогноз и оценка процентов не рассчитываются: методика и необходимые исходные данные не согласованы.",
            ));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .border_t_1()
            .border_color(border)
            .pt_2()
            .when_some(self.tracker.current_report(), |footer, summary| {
                footer.child(
                    div()
                        .id("analysis-pie-scroll")
                        .max_h(px(CHARTS_MAX_HEIGHT))
                        .overflow_y_scroll()
                        .child(Self::render_pie_charts(summary)),
                )
            })
            .child(notes)
    }
}

impl Render for AnalysisPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let stripe = cx.theme().muted;
        let border = cx.theme().border;
        let mut root = div().size_full().flex().flex_col().gap_2().p_3();
        root = root.child(self.render_filters(cx));
        if let Some(message) = &self.message {
            root = root.child(div().text_color(danger).child(message.clone()));
        }
        let body = if let Some(error) = self.tracker.current_error() {
            div()
                .flex_1()
                .text_color(danger)
                .child(error.to_owned())
                .into_any_element()
        } else if let Some(summary) = self.tracker.current_report() {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(Self::render_summary(summary, stripe, border))
                .into_any_element()
        } else if self.tracker.loading {
            div()
                .flex_1()
                .child("Загружаем операции…")
                .into_any_element()
        } else {
            div().flex_1().into_any_element()
        };
        root.child(body).child(self.render_footer(cx))
    }
}

fn category_tree_labels(categories: &[Category]) -> Vec<(i64, String)> {
    super::support::category_tree_labels(categories)
}

#[cfg(test)]
mod tests {
    // Глоб-импорт `gpui_kit::*` затеняет атрибут `#[test]`, поэтому имена импортируются явно.
    use super::{
        parse_filters, pie_chart_slices, pie_slice_points, AnalysisFilters, CategoryPicker,
        RequestTracker,
    };
    use chrono::NaiveDate;
    use finance_core::{
        AnalysisCategoryTotal, AnalysisSummary, AnalysisTotal, Category, Currency, TransactionKind,
    };
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
        })
        .collect();
        AnalysisSummary { categories, totals }
    }

    fn filters(from: &str, to: &str, income: Option<i64>, expense: Option<i64>) -> AnalysisFilters {
        AnalysisFilters {
            from: from.to_owned(),
            to: to.to_owned(),
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
    fn empty_analysis_summary_has_no_pie_chart() {
        let summary = empty_summary();
        let slices = pie_chart_slices(&summary, TransactionKind::Income, Currency::Rub)
            .expect("empty summary is representable");
        assert!(slices.is_empty());
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
    fn stale_analysis_result_does_not_replace_current_filter_result() {
        let mut tracker = RequestTracker::default();
        let previous = filters("", "", None, None);
        let previous_generation = tracker.start(previous.clone());
        let current = filters("2027-01-01", "", Some(2), Some(1));
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

        tracker.start(filters("2027-01-01", "", None, None));

        assert!(tracker.current_report().is_none());
    }

    #[test]
    fn analysis_filter_rejects_reversed_period_boundaries() {
        assert_eq!(
            parse_filters(&filters("2027-02-01", "2027-01-31", None, None)),
            Err("Начало периода не может быть позже его окончания".to_owned())
        );
    }

    #[test]
    fn analysis_filter_parses_dates_and_categories() {
        let parsed = parse_filters(&filters("2027-01-01", "", Some(3), None))
            .expect("valid filters should parse");
        assert_eq!(parsed.from, NaiveDate::from_ymd_opt(2027, 1, 1));
        assert_eq!(parsed.to, None);
        assert_eq!(parsed.income_category_id, Some(3));
        assert_eq!(parsed.expense_category_id, None);
    }

    #[test]
    fn analysis_filter_rejects_malformed_dates() {
        assert_eq!(
            parse_filters(&filters("01.01.2027", "", None, None)),
            Err("Введите дату начала периода в формате ГГГГ-ММ-ДД".to_owned())
        );
        assert_eq!(
            parse_filters(&filters("", "вчера", None, None)),
            Err("Введите дату окончания периода в формате ГГГГ-ММ-ДД".to_owned())
        );
    }

    #[test]
    fn category_picker_maps_confirmed_label_to_category_id() {
        let categories = vec![
            Category {
                id: 7,
                name: "Зарплата".to_owned(),
                parent_id: None,
                is_active: true,
            },
            Category {
                id: 9,
                name: "Премия".to_owned(),
                parent_id: Some(7),
                is_active: true,
            },
        ];
        let (ids, labels) = CategoryPicker::labels_for(&categories);
        assert_eq!(ids, [7, 9]);
        assert_eq!(labels, ["Все", "Зарплата", "Зарплата > Премия"]);
    }
}
