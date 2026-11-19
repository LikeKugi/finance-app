use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use eframe::egui;
use finance_core::{summarize_analysis, AnalysisSummary, Category, Currency, TransactionKind};

use super::date_input::{date_input, DateInputState};

const ANALYSIS_COLUMN_MIN_WIDTHS: [f32; 4] = [170.0, 75.0, 120.0, 75.0];
const ANALYSIS_COLUMN_GROWTH_WEIGHTS: [f32; 4] = [2.5, 0.8, 1.5, 0.7];
const ANALYSIS_ROW_HEIGHT: f32 = 30.0;

fn analysis_column_widths(available_width: f32, column_spacing: f32) -> [f32; 4] {
    let content_width = (available_width - column_spacing * 3.0).max(0.0);
    let minimum_width: f32 = ANALYSIS_COLUMN_MIN_WIDTHS.iter().sum();
    if content_width < minimum_width {
        let scale = content_width / minimum_width;
        return std::array::from_fn(|index| ANALYSIS_COLUMN_MIN_WIDTHS[index] * scale);
    }

    let extra_width = content_width - minimum_width;
    let total_growth_weight: f32 = ANALYSIS_COLUMN_GROWTH_WEIGHTS.iter().sum();
    std::array::from_fn(|index| {
        ANALYSIS_COLUMN_MIN_WIDTHS[index]
            + extra_width * ANALYSIS_COLUMN_GROWTH_WEIGHTS[index] / total_growth_weight
    })
}

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

struct ScheduledRequest {
    generation: u64,
    filters: AnalysisFilters,
    starts_at: Instant,
}

struct PendingRequest {
    generation: u64,
    filters: AnalysisFilters,
    receiver: Receiver<Result<AnalysisSummary, String>>,
}

#[derive(Default)]
pub(super) struct AnalysisPage {
    from_date: String,
    to_date: String,
    income_category_id: Option<i64>,
    expense_category_id: Option<i64>,
    applied_filters: Option<AnalysisFilters>,
    from_date_input: DateInputState,
    to_date_input: DateInputState,
    observed_filters: Option<AnalysisFilters>,
    scheduled_request: Option<ScheduledRequest>,
    pending_request: Option<PendingRequest>,
    generation: u64,
    report: Option<(AnalysisFilters, AnalysisSummary)>,
    error: Option<(AnalysisFilters, String)>,
}

impl AnalysisPage {
    pub(super) fn render(
        &mut self,
        context: &egui::Context,
        ui: &mut egui::Ui,
        database_path: Option<&Path>,
        income_categories: &[Category],
        expense_categories: &[Category],
        refresh: bool,
    ) -> bool {
        let refresh_categories = refresh;
        let mut apply_filters = false;
        let mut reset_filters = false;
        let mut clear_from_date = false;
        let mut clear_to_date = false;
        let mut clear_income_category = false;
        let mut clear_expense_category = false;
        let filter_height = ui.spacing().interact_size.y;
        ui.horizontal_wrapped(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(252.0, filter_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label("Период с");
                    date_input(
                        ui,
                        "analysis_from_date",
                        &mut self.from_date,
                        &mut self.from_date_input,
                        112.0,
                    );
                    if ui
                        .add_enabled(!self.from_date.trim().is_empty(), egui::Button::new("×"))
                        .on_hover_text("Очистить начальную дату")
                        .clicked()
                    {
                        clear_from_date = true;
                    }
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(252.0, filter_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label("по");
                    date_input(
                        ui,
                        "analysis_to_date",
                        &mut self.to_date,
                        &mut self.to_date_input,
                        112.0,
                    );
                    if ui
                        .add_enabled(!self.to_date.trim().is_empty(), egui::Button::new("×"))
                        .on_hover_text("Очистить конечную дату")
                        .clicked()
                    {
                        clear_to_date = true;
                    }
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(420.0, filter_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label("Категория доходов:");
                    Self::render_category_filter(
                        ui,
                        "analysis_income_category_filter",
                        &mut self.income_category_id,
                        income_categories,
                    );
                    if ui
                        .add_enabled(self.income_category_id.is_some(), egui::Button::new("×"))
                        .on_hover_text("Очистить категорию доходов")
                        .clicked()
                    {
                        clear_income_category = true;
                    }
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(420.0, filter_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label("Категория расходов:");
                    Self::render_category_filter(
                        ui,
                        "analysis_expense_category_filter",
                        &mut self.expense_category_id,
                        expense_categories,
                    );
                    if ui
                        .add_enabled(self.expense_category_id.is_some(), egui::Button::new("×"))
                        .on_hover_text("Очистить категорию расходов")
                        .clicked()
                    {
                        clear_expense_category = true;
                    }
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(230.0, filter_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if ui.button("Применить").clicked() {
                        apply_filters = true;
                    }
                    if ui
                        .add_enabled(
                            self.has_active_filters(),
                            egui::Button::new("Сбросить фильтры"),
                        )
                        .clicked()
                    {
                        reset_filters = true;
                    }
                },
            );
        });
        ui.separator();

        if reset_filters {
            self.reset_filters();
            self.apply_pending_filters();
        } else if clear_from_date {
            self.clear_from_date_filter();
        } else if clear_to_date {
            self.clear_to_date_filter();
        } else if clear_income_category {
            self.clear_income_category_filter();
        } else if clear_expense_category {
            self.clear_expense_category_filter();
        } else if apply_filters || refresh || self.applied_filters.is_none() {
            self.apply_pending_filters();
        }
        let filters = self
            .applied_filters
            .clone()
            .unwrap_or_else(|| self.filters());
        if let Some(database_path) = database_path {
            self.poll_request(&filters);
            self.start_scheduled_request(context, database_path);
        }

        if let Some((error_filters, error)) = &self.error {
            if error_filters == &filters {
                ui.colored_label(egui::Color32::RED, error);
                return refresh_categories;
            }
        }

        if let Some((report_filters, summary)) = &self.report {
            if report_filters == &filters {
                Self::render_summary(ui, summary);
                return refresh_categories;
            }
        }

        if database_path.is_none() {
            ui.colored_label(
                egui::Color32::RED,
                "Для анализа требуется путь к локальной базе данных.",
            );
        } else if self.pending_request.is_some() || self.scheduled_request.is_some() {
            ui.spinner();
            ui.label("Загружаем операции…");
            context.request_repaint_after(Duration::from_millis(100));
        }
        refresh_categories
    }

    pub(super) fn render_footer(&self, ui: &mut egui::Ui) {
        ui.label(
            "Доли считаются отдельно для доходов и расходов и только внутри одной валюты. RUB и USD не пересчитываются.",
        );
        ui.colored_label(
            egui::Color32::from_rgb(160, 105, 0),
            "Прогноз и оценка процентов не рассчитываются: методика и необходимые исходные данные не согласованы.",
        );
    }

    fn filters(&self) -> AnalysisFilters {
        AnalysisFilters {
            from: self.from_date.trim().to_owned(),
            to: self.to_date.trim().to_owned(),
            income_category_id: self.income_category_id,
            expense_category_id: self.expense_category_id,
        }
    }

    fn has_active_filters(&self) -> bool {
        let filters = self.filters();
        !filters.from.is_empty()
            || !filters.to.is_empty()
            || filters.income_category_id.is_some()
            || filters.expense_category_id.is_some()
    }

    fn apply_pending_filters(&mut self) {
        let filters = self.filters();
        self.applied_filters = Some(filters.clone());
        self.schedule(filters, Instant::now());
    }

    fn clear_from_date_filter(&mut self) {
        self.from_date.clear();
        self.from_date_input.reset();
        self.apply_pending_filters();
    }

    fn clear_to_date_filter(&mut self) {
        self.to_date.clear();
        self.to_date_input.reset();
        self.apply_pending_filters();
    }

    fn clear_income_category_filter(&mut self) {
        self.income_category_id = None;
        self.apply_pending_filters();
    }

    fn clear_expense_category_filter(&mut self) {
        self.expense_category_id = None;
        self.apply_pending_filters();
    }

    fn reset_filters(&mut self) {
        self.from_date.clear();
        self.to_date.clear();
        self.income_category_id = None;
        self.expense_category_id = None;
        self.from_date_input.reset();
        self.to_date_input.reset();
    }

    fn render_category_filter(
        ui: &mut egui::Ui,
        id_salt: &str,
        selected_category: &mut Option<i64>,
        categories: &[Category],
    ) {
        let category_labels = super::category_tree_labels(categories);
        let selected_name = selected_category.and_then(|id| {
            category_labels
                .iter()
                .find(|(category_id, _)| *category_id == id)
                .map(|(_, label)| label.as_str())
        });
        if selected_category.is_some() && selected_name.is_none() {
            *selected_category = None;
        }
        let selected_name = selected_name.unwrap_or("Все");
        egui::ComboBox::from_id_salt(id_salt)
            .width(220.0)
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                ui.selectable_value(selected_category, None, "Все");
                for (category_id, label) in &category_labels {
                    ui.selectable_value(selected_category, Some(*category_id), label);
                }
            });
    }

    fn schedule(&mut self, filters: AnalysisFilters, starts_at: Instant) {
        self.generation = self.generation.wrapping_add(1);
        self.observed_filters = Some(filters.clone());
        self.scheduled_request = Some(ScheduledRequest {
            generation: self.generation,
            filters,
            starts_at,
        });
        self.error = None;
    }

    fn poll_request(&mut self, current_filters: &AnalysisFilters) {
        let Some(request) = &self.pending_request else {
            return;
        };

        let generation = request.generation;
        let request_filters = request.filters.clone();
        let received = match request.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(
                "Не удалось получить результат загрузки отчёта".to_owned(),
            )),
        };

        if let Some(result) = received {
            self.pending_request = None;
            self.accept_result(generation, request_filters, result, current_filters);
        }
    }

    fn accept_result(
        &mut self,
        generation: u64,
        request_filters: AnalysisFilters,
        result: Result<AnalysisSummary, String>,
        current_filters: &AnalysisFilters,
    ) {
        if generation != self.generation
            || self.observed_filters.as_ref() != Some(current_filters)
            || &request_filters != current_filters
        {
            return;
        }

        match result {
            Ok(summary) => self.report = Some((request_filters, summary)),
            Err(error) => self.error = Some((request_filters, error)),
        }
    }

    fn start_scheduled_request(&mut self, context: &egui::Context, database_path: &Path) {
        let Some(request) = &self.scheduled_request else {
            return;
        };
        if Instant::now() < request.starts_at {
            context
                .request_repaint_after(request.starts_at.saturating_duration_since(Instant::now()));
            return;
        }

        let Some(request) = self.scheduled_request.take() else {
            return;
        };
        let parsed = match parse_filters(&request.filters) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.error = Some((request.filters, error));
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        let path = database_path.to_owned();
        let repaint_context = context.clone();
        let generation = request.generation;
        let filters = request.filters;
        let spawn_result = thread::Builder::new()
            .name("finance-analysis".to_owned())
            .spawn(move || {
                let ParsedFilters {
                    from,
                    to,
                    income_category_id,
                    expense_category_id,
                } = parsed;
                let result = finance_db::load_analysis_transactions_by_category(
                    &path,
                    from,
                    to,
                    income_category_id,
                    expense_category_id,
                )
                .map_err(|error| error.to_string())
                .and_then(|transactions| {
                    summarize_analysis(&transactions).map_err(|error| error.to_string())
                });
                let _ = sender.send(result);
                repaint_context.request_repaint();
            });

        match spawn_result {
            Ok(_) => {
                self.pending_request = Some(PendingRequest {
                    generation,
                    filters,
                    receiver,
                });
            }
            Err(_) => {
                self.error = Some((
                    filters,
                    "Не удалось запустить фоновую загрузку отчёта".to_owned(),
                ));
            }
        }
    }

    fn render_summary(ui: &mut egui::Ui, summary: &AnalysisSummary) {
        if summary.totals.is_empty() {
            ui.label("За выбранный период операций нет.");
            return;
        }

        ui.columns(2, |columns| {
            for (column, (kind, scroll_id)) in columns.iter_mut().zip([
                (TransactionKind::Income, "analysis_income_summary_scroll"),
                (TransactionKind::Expense, "analysis_expense_summary_scroll"),
            ]) {
                egui::ScrollArea::vertical()
                    .id_salt(scroll_id)
                    .auto_shrink([false, true])
                    .show(column, |ui| Self::render_kind_summary(ui, summary, kind));
            }
        });
    }

    fn render_kind_summary(ui: &mut egui::Ui, summary: &AnalysisSummary, kind: TransactionKind) {
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

            ui.heading(format!("{} — {}", kind.title(), currency.code()));
            let column_spacing = ui.spacing().item_spacing.x;
            let column_widths = analysis_column_widths(ui.available_width(), column_spacing);
            Self::render_summary_row(
                ui,
                &column_widths,
                ["Категория", "Операций", "Сумма", "Доля"],
                true,
                false,
            );

            for (row_index, category) in summary
                .categories
                .iter()
                .filter(|category| category.kind == kind && category.currency == currency)
                .enumerate()
            {
                let transaction_count = category.transaction_count.to_string();
                let amount = category.amount.to_string();
                let share_percent = format!("{}%", category.share_percent.round_dp(2));
                Self::render_summary_row(
                    ui,
                    &column_widths,
                    [
                        &category.category_name,
                        &transaction_count,
                        &amount,
                        &share_percent,
                    ],
                    false,
                    row_index % 2 == 1,
                );
            }

            let transaction_count = total.transaction_count.to_string();
            let amount = total.amount.to_string();
            Self::render_summary_row(
                ui,
                &column_widths,
                ["Итого", &transaction_count, &amount, "100%"],
                true,
                false,
            );
            ui.add_space(12.0);
        }

        if !has_totals {
            ui.label(format!(
                "За выбранный период операций типа «{}» нет.",
                kind.title()
            ));
        }
    }

    fn render_summary_row(
        ui: &mut egui::Ui,
        column_widths: &[f32; 4],
        values: [&str; 4],
        strong: bool,
        striped: bool,
    ) {
        let column_spacing = ui.spacing().item_spacing.x;
        let row_width = column_widths.iter().sum::<f32>() + column_spacing * 3.0;
        ui.allocate_ui_with_layout(
            egui::vec2(row_width, ANALYSIS_ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |row_ui| {
                if striped {
                    row_ui.painter().rect_filled(
                        row_ui.max_rect(),
                        0.0,
                        row_ui.visuals().faint_bg_color,
                    );
                }

                for (column_index, (value, width)) in
                    values.into_iter().zip(column_widths).enumerate()
                {
                    let cell_layout = if column_index == 0 {
                        egui::Layout::left_to_right(egui::Align::Center)
                    } else {
                        egui::Layout::right_to_left(egui::Align::Center)
                    };
                    row_ui.allocate_ui_with_layout(
                        egui::vec2(*width, ANALYSIS_ROW_HEIGHT),
                        cell_layout,
                        |cell_ui| {
                            let text = if strong {
                                egui::RichText::new(value).strong()
                            } else {
                                egui::RichText::new(value)
                            };
                            cell_ui
                                .add(egui::Label::new(text).truncate())
                                .on_hover_text(value);
                        },
                    );
                }
            },
        );
    }
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
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(Some)
        .map_err(|_| format!("Введите дату {field_name} периода в формате ГГГГ-ММ-ДД"))
}

#[cfg(test)]
mod tests {
    use super::{parse_filters, AnalysisFilters, AnalysisPage};
    use eframe::egui;
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
            share_percent: Decimal::ONE_HUNDRED,
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
            amount: Decimal::ONE,
            transaction_count: 1,
        })
        .collect();

        AnalysisSummary { categories, totals }
    }

    fn render_summary_at_width(summary: &AnalysisSummary, width: f32) -> egui::FullOutput {
        let context = egui::Context::default();
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 720.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        AnalysisPage::render_summary(ui, summary);
                    });
                },
            ));
        }
        output.expect("analysis summary should render twice")
    }

    fn text_position(output: &egui::FullOutput, text: &str) -> Option<egui::Pos2> {
        output
            .shapes
            .iter()
            .find_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) if text_shape.galley.job.text == text => {
                    Some(text_shape.pos)
                }
                _ => None,
            })
    }

    fn has_text(output: &egui::FullOutput, text: &str) -> bool {
        text_position(output, text).is_some()
    }

    fn text_count(output: &egui::FullOutput, text: &str) -> usize {
        output
            .shapes
            .iter()
            .filter(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) => text_shape.galley.job.text == text,
                _ => false,
            })
            .count()
    }

    fn text_position_on_row(
        output: &egui::FullOutput,
        text: &str,
        row_y: f32,
        reference_x: f32,
        split_x: f32,
    ) -> Option<egui::Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape)
                    if text_shape.galley.job.text == text
                        && (text_shape.pos.y - row_y).abs() < 1.0
                        && (text_shape.pos.x < split_x) == (reference_x < split_x) =>
                {
                    Some(text_shape.pos)
                }
                _ => None,
            })
            .min_by(|left, right| {
                (left.x - reference_x)
                    .abs()
                    .total_cmp(&(right.x - reference_x).abs())
            })
    }

    fn striped_row_width_near(output: &egui::FullOutput, row_y: f32) -> Option<f32> {
        let faint_bg_color = egui::Context::default().style().visuals.faint_bg_color;
        output
            .shapes
            .iter()
            .filter_map(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Rect(rect_shape)
                    if rect_shape.fill == faint_bg_color
                        && (rect_shape.rect.center().y - row_y).abs() < 20.0 =>
                {
                    Some(rect_shape.rect.width())
                }
                _ => None,
            })
            .max_by(f32::total_cmp)
    }

    fn page_with_selected_filters() -> AnalysisPage {
        AnalysisPage {
            from_date: "2027-01-01".to_owned(),
            to_date: "2027-01-31".to_owned(),
            income_category_id: Some(1),
            expense_category_id: Some(2),
            ..AnalysisPage::default()
        }
    }

    fn render_analysis_page_at_width(width: f32) -> egui::FullOutput {
        let mut page = page_with_selected_filters();
        let income_categories = vec![Category {
            id: 1,
            name: "Зарплата".to_owned(),
            parent_id: None,
            is_active: true,
        }];
        let expense_categories = vec![Category {
            id: 2,
            name: "Покупки".to_owned(),
            parent_id: None,
            is_active: true,
        }];
        let context = egui::Context::default();
        let mut output = None;
        for _ in 0..2 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 720.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        page.render(
                            context,
                            ui,
                            None,
                            &income_categories,
                            &expense_categories,
                            false,
                        );
                    });
                },
            ));
        }
        output.expect("analysis page should render twice")
    }

    #[test]
    fn stale_analysis_result_does_not_replace_current_filter_result() {
        let mut page = AnalysisPage::default();
        let previous_filters = page.filters();
        page.schedule(previous_filters.clone(), std::time::Instant::now());
        let previous_generation = page.generation;
        let current_filters = AnalysisFilters {
            from: "2027-01-01".to_owned(),
            to: String::new(),
            income_category_id: Some(2),
            expense_category_id: Some(1),
        };
        page.schedule(current_filters.clone(), std::time::Instant::now());

        page.accept_result(
            previous_generation,
            previous_filters,
            Ok(AnalysisSummary {
                categories: Vec::new(),
                totals: Vec::new(),
            }),
            &current_filters,
        );

        assert!(page.report.is_none());
    }

    #[test]
    fn analysis_filters_default_to_all_income_and_expense_categories() {
        let filters = AnalysisPage::default().filters();

        assert_eq!(filters.income_category_id, None);
        assert_eq!(filters.expense_category_id, None);
    }

    #[test]
    fn analysis_filter_edits_wait_until_apply_is_requested() {
        let mut page = AnalysisPage::default();
        page.apply_pending_filters();
        let initial_generation = page.generation;
        let initial_filters = page.applied_filters.clone();
        let income_categories = vec![Category {
            id: 3,
            name: "Зарплата".to_owned(),
            parent_id: None,
            is_active: true,
        }];
        let context = egui::Context::default();

        page.from_date = "2027-01-01".to_owned();
        page.income_category_id = Some(3);
        let _output = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                page.render(context, ui, None, &income_categories, &[], false);
            });
        });

        assert_eq!(page.generation, initial_generation);
        assert_eq!(page.applied_filters, initial_filters);

        page.apply_pending_filters();

        assert_eq!(page.applied_filters, Some(page.filters()));
        assert_eq!(page.generation, initial_generation + 1);
    }

    #[test]
    fn clearing_analysis_start_date_applies_other_pending_filters() {
        let mut page = page_with_selected_filters();

        page.clear_from_date_filter();

        assert_eq!(page.applied_filters, Some(page.filters()));
        assert!(page.from_date.is_empty());
        assert_eq!(page.to_date, "2027-01-31");
        assert_eq!(page.income_category_id, Some(1));
        assert_eq!(page.expense_category_id, Some(2));
    }

    #[test]
    fn clearing_analysis_end_date_preserves_other_filters() {
        let mut page = page_with_selected_filters();

        page.clear_to_date_filter();

        assert_eq!(page.applied_filters, Some(page.filters()));
        assert!(page.to_date.is_empty());
        assert_eq!(page.from_date, "2027-01-01");
        assert_eq!(page.income_category_id, Some(1));
        assert_eq!(page.expense_category_id, Some(2));
    }

    #[test]
    fn clearing_analysis_income_category_preserves_other_filters() {
        let mut page = page_with_selected_filters();

        page.clear_income_category_filter();

        assert_eq!(page.applied_filters, Some(page.filters()));
        assert!(page.income_category_id.is_none());
        assert_eq!(page.expense_category_id, Some(2));
        assert_eq!(page.from_date, "2027-01-01");
        assert_eq!(page.to_date, "2027-01-31");
    }

    #[test]
    fn clearing_analysis_expense_category_preserves_other_filters() {
        let mut page = page_with_selected_filters();

        page.clear_expense_category_filter();

        assert_eq!(page.applied_filters, Some(page.filters()));
        assert!(page.expense_category_id.is_none());
        assert_eq!(page.income_category_id, Some(1));
        assert_eq!(page.from_date, "2027-01-01");
        assert_eq!(page.to_date, "2027-01-31");
    }

    #[test]
    fn resetting_analysis_filters_clears_all_values() {
        let mut page = page_with_selected_filters();

        page.reset_filters();
        page.apply_pending_filters();

        assert_eq!(page.applied_filters, Some(page.filters()));
        assert!(!page.has_active_filters());
    }

    #[test]
    fn analysis_filter_controls_render_apply_reset_and_one_clear_button_per_filter() {
        let mut page = page_with_selected_filters();
        let income_categories = vec![Category {
            id: 1,
            name: "Зарплата".to_owned(),
            parent_id: None,
            is_active: true,
        }];
        let expense_categories = vec![Category {
            id: 2,
            name: "Покупки".to_owned(),
            parent_id: None,
            is_active: true,
        }];
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(2_000.0, 720.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    page.render(
                        context,
                        ui,
                        None,
                        &income_categories,
                        &expense_categories,
                        false,
                    );
                });
            },
        );

        assert_eq!(text_count(&output, "×"), 4);
        assert!(has_text(&output, "Применить"));
        assert!(has_text(&output, "Сбросить фильтры"));
        assert!(!has_text(&output, "Очистить период"));
    }

    #[test]
    fn analysis_filters_wrap_when_available_width_is_limited() {
        let wide_output = render_analysis_page_at_width(2_000.0);
        let narrow_output = render_analysis_page_at_width(900.0);
        let filter_labels = [
            "Период с",
            "по",
            "Категория доходов:",
            "Категория расходов:",
            "Применить",
            "Сбросить фильтры",
        ];

        for (width, output) in [(2_000.0, &wide_output), (900.0, &narrow_output)] {
            for label in filter_labels {
                assert!(
                    has_text(output, label),
                    "missing filter control at width {width}: {label}"
                );
            }
            assert!(has_text(output, "2027-01-01"));
            assert!(has_text(output, "2027-01-31"));
            assert_eq!(text_count(output, "×"), 4);
        }

        let wide_positions: Vec<_> = [
            "2027-01-01",
            "2027-01-31",
            "Зарплата",
            "Покупки",
            "Применить",
            "Сбросить фильтры",
        ]
        .iter()
        .map(|label| text_position(&wide_output, label).expect("filter label"))
        .collect();
        let min_y = wide_positions
            .iter()
            .map(|position| position.y)
            .fold(f32::INFINITY, f32::min);
        let max_y = wide_positions
            .iter()
            .map(|position| position.y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(max_y - min_y < 6.0);

        let start_y = text_position(&narrow_output, "2027-01-01")
            .expect("start date filter")
            .y;
        let expense_category_y = text_position(&narrow_output, "Покупки")
            .expect("expense category selection")
            .y;
        assert!(expense_category_y > start_y);
    }

    #[test]
    fn analysis_category_filters_keep_selections_independent() {
        let income_filter = AnalysisPage {
            income_category_id: Some(4),
            ..AnalysisPage::default()
        }
        .filters();
        let both_filters = AnalysisPage {
            income_category_id: Some(4),
            expense_category_id: Some(4),
            ..AnalysisPage::default()
        }
        .filters();

        assert_eq!(
            income_filter.income_category_id,
            both_filters.income_category_id
        );
        assert_eq!(income_filter.expense_category_id, None);
        assert_eq!(both_filters.expense_category_id, Some(4));
        let parsed = super::parse_filters(&both_filters).unwrap();
        assert_eq!(parsed.income_category_id, Some(4));
        assert_eq!(parsed.expense_category_id, Some(4));
    }

    #[test]
    fn analysis_category_selects_use_their_own_category_lists() {
        let income_categories = vec![Category {
            id: 1,
            name: "Зарплата".to_owned(),
            parent_id: None,
            is_active: false,
        }];
        let expense_categories = vec![Category {
            id: 1,
            name: "Покупки".to_owned(),
            parent_id: None,
            is_active: true,
        }];
        let mut page = AnalysisPage {
            income_category_id: Some(1),
            expense_category_id: Some(1),
            ..AnalysisPage::default()
        };
        let context = egui::Context::default();
        let output = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                page.render(
                    context,
                    ui,
                    None,
                    &income_categories,
                    &expense_categories,
                    false,
                );
            });
        });

        assert!(has_text(&output, "Зарплата"));
        assert!(has_text(&output, "Покупки"));
    }

    #[test]
    fn analysis_category_select_resets_a_removed_category_to_all() {
        let mut selected_category = Some(404);
        let context = egui::Context::default();
        let _output = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                AnalysisPage::render_category_filter(
                    ui,
                    "analysis_removed_category_filter",
                    &mut selected_category,
                    &[],
                );
            });
        });

        assert_eq!(selected_category, None);
    }

    #[test]
    fn analysis_filter_rejects_reversed_period_boundaries() {
        let filters = AnalysisFilters {
            from: "2027-02-01".to_owned(),
            to: "2027-01-31".to_owned(),
            income_category_id: None,
            expense_category_id: None,
        };

        assert_eq!(
            parse_filters(&filters),
            Err("Начало периода не может быть позже его окончания".to_owned())
        );
    }

    #[test]
    fn analysis_renders_income_and_expenses_in_separate_columns() {
        let summary = summary_with_both_kinds_and_currencies();
        let output = render_summary_at_width(&summary, 1_280.0);

        assert!(!output
            .shapes
            .iter()
            .any(|clipped_shape| match &clipped_shape.shape {
                egui::epaint::Shape::Text(text_shape) => {
                    text_shape.galley.job.text.contains("ScrollArea ID")
                }
                _ => false,
            }));

        let income_rub = text_position(&output, "Доходы — RUB").expect("income heading");
        let income_usd = text_position(&output, "Доходы — USD").expect("income USD heading");
        let expense_rub = text_position(&output, "Расходы — RUB").expect("expense heading");
        let expense_usd = text_position(&output, "Расходы — USD").expect("expense USD heading");

        assert!(income_rub.x < expense_rub.x);
        assert!(income_usd.x < expense_usd.x);
        assert_eq!(income_rub.x, income_usd.x);
        assert_eq!(expense_rub.x, expense_usd.x);
    }

    #[test]
    fn analysis_keeps_both_groups_visible_together() {
        let summary = summary_with_both_kinds_and_currencies();
        let output = render_summary_at_width(&summary, 1_280.0);

        assert!(has_text(&output, "Доходы — RUB"));
        assert!(has_text(&output, "Расходы — RUB"));
        assert!(has_text(&output, "Зарплата"));
        assert!(has_text(&output, "Покупки"));
    }

    #[test]
    fn analysis_summary_columns_and_stripes_fill_each_section_at_different_widths() {
        let summary = summary_with_both_kinds_and_currencies();
        let category_rows = ["Зарплата", "Доход USD", "Покупки", "Расход USD"];
        let striped_rows = ["Премия", "Бонус USD", "Коммунальные услуги", "Платеж USD"];

        for width in [900.0, 1_280.0, 2_000.0] {
            let output = render_summary_at_width(&summary, width);
            for (category, striped_category) in category_rows.into_iter().zip(striped_rows) {
                let category_position = text_position(&output, category).expect("category row");
                let share_position = text_position_on_row(
                    &output,
                    "100%",
                    category_position.y,
                    category_position.x,
                    width / 2.0,
                )
                .expect("share cell");
                assert!(
                    share_position.x - category_position.x > width * 0.3,
                    "last column should use the section width at {width}px for {category}: category x={}, share x={}",
                    category_position.x,
                    share_position.x
                );

                let striped_position =
                    text_position(&output, striped_category).expect("striped category row");
                let background_width = striped_row_width_near(&output, striped_position.y)
                    .expect("striped row background");
                assert!(
                    background_width > width * 0.4,
                    "striped row should span the section width at {width}px for {striped_category}"
                );
            }
        }
    }

    #[test]
    fn analysis_shows_empty_group_message_when_only_one_kind_has_data() {
        let mut summary = summary_with_both_kinds_and_currencies();
        summary
            .categories
            .retain(|category| category.kind == TransactionKind::Income);
        summary
            .totals
            .retain(|total| total.kind == TransactionKind::Income);
        let output = render_summary_at_width(&summary, 1_280.0);

        assert!(has_text(&output, "Доходы — RUB"));
        assert!(has_text(
            &output,
            "За выбранный период операций типа «Расходы» нет."
        ));
    }

    #[test]
    fn empty_analysis_summary_keeps_the_no_operations_message() {
        let summary = AnalysisSummary {
            categories: Vec::new(),
            totals: Vec::new(),
        };
        let output = render_summary_at_width(&summary, 1_280.0);

        assert!(has_text(&output, "За выбранный период операций нет."));
    }
}
