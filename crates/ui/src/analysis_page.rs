use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use eframe::egui;
use finance_core::{summarize_analysis, AnalysisSummary, Currency, TransactionKind};

use super::date_input::{date_input, DateInputState};

const FILTER_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalysisMode {
    All,
    Income,
    Expenses,
}

impl AnalysisMode {
    const ALL: [Self; 3] = [Self::All, Self::Income, Self::Expenses];

    const fn title(self) -> &'static str {
        match self {
            Self::All => "Все операции",
            Self::Income => "Доходы",
            Self::Expenses => "Расходы",
        }
    }

    const fn transaction_kind(self) -> Option<TransactionKind> {
        match self {
            Self::All => None,
            Self::Income => Some(TransactionKind::Income),
            Self::Expenses => Some(TransactionKind::Expense),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalysisFilters {
    from: String,
    to: String,
    mode: AnalysisMode,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedFilters {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    kind_filter: Option<TransactionKind>,
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
    mode: Option<AnalysisMode>,
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
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Период с");
            date_input(
                ui,
                "analysis_from_date",
                &mut self.from_date,
                &mut self.from_date_input,
                112.0,
            );
            ui.label("по");
            date_input(
                ui,
                "analysis_to_date",
                &mut self.to_date,
                &mut self.to_date_input,
                112.0,
            );
            ui.label("Режим");
            egui::ComboBox::from_id_salt("analysis_mode")
                .selected_text(self.mode().title())
                .show_ui(ui, |ui| {
                    for mode in AnalysisMode::ALL {
                        ui.selectable_value(&mut self.mode, Some(mode), mode.title());
                    }
                });
            if ui.button("Очистить период").clicked() {
                self.from_date.clear();
                self.to_date.clear();
                self.from_date_input.reset();
                self.to_date_input.reset();
            }
            let refresh_clicked = ui.button("↻ Обновить").clicked();
            if refresh_clicked {
                let filters = self.filters();
                self.schedule(filters, Instant::now());
            }
        });

        let filters = self.filters();
        self.observe_filters(filters.clone());
        if let Some(database_path) = database_path {
            self.poll_request(&filters);
            self.start_scheduled_request(context, database_path);
        }

        if let Some((error_filters, error)) = &self.error {
            if error_filters == &filters {
                ui.colored_label(egui::Color32::RED, error);
                return;
            }
        }

        if let Some((report_filters, summary)) = &self.report {
            if report_filters == &filters {
                Self::render_summary(ui, summary, filters.mode);
                return;
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

    fn mode(&self) -> AnalysisMode {
        self.mode.unwrap_or(AnalysisMode::All)
    }

    fn filters(&self) -> AnalysisFilters {
        AnalysisFilters {
            from: self.from_date.trim().to_owned(),
            to: self.to_date.trim().to_owned(),
            mode: self.mode(),
        }
    }

    fn observe_filters(&mut self, filters: AnalysisFilters) {
        if self.observed_filters.as_ref() != Some(&filters) {
            self.schedule(filters, Instant::now() + FILTER_DEBOUNCE);
        }
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
                    kind_filter,
                } = parsed;
                let result = finance_db::load_analysis_transactions(&path, from, to, kind_filter)
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

    fn render_summary(ui: &mut egui::Ui, summary: &AnalysisSummary, mode: AnalysisMode) {
        if summary.totals.is_empty() {
            ui.label("За выбранный период операций нет.");
            return;
        }

        if mode == AnalysisMode::All {
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
        } else if let Some(kind) = mode.transaction_kind() {
            egui::ScrollArea::vertical().show(ui, |ui| {
                Self::render_kind_summary(ui, summary, kind);
            });
        }
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
            egui::Grid::new(format!("analysis_{kind:?}_{}", currency.code()))
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Категория");
                    ui.strong("Операций");
                    ui.strong("Сумма");
                    ui.strong("Доля");
                    ui.end_row();

                    for category in summary
                        .categories
                        .iter()
                        .filter(|category| category.kind == kind && category.currency == currency)
                    {
                        ui.label(&category.category_name);
                        ui.label(category.transaction_count.to_string());
                        ui.label(category.amount.to_string());
                        ui.label(format!("{}%", category.share_percent.round_dp(2)));
                        ui.end_row();
                    }

                    ui.strong("Итого");
                    ui.strong(total.transaction_count.to_string());
                    ui.strong(total.amount.to_string());
                    ui.strong("100%");
                    ui.end_row();
                });
            ui.add_space(12.0);
        }

        if !has_totals {
            ui.label(format!(
                "За выбранный период операций типа «{}» нет.",
                kind.title()
            ));
        }
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
        kind_filter: filters.mode.transaction_kind(),
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
    use super::{parse_filters, AnalysisFilters, AnalysisMode, AnalysisPage};
    use eframe::egui;
    use finance_core::{
        AnalysisCategoryTotal, AnalysisSummary, AnalysisTotal, Currency, TransactionKind,
    };
    use rust_decimal::Decimal;

    fn summary_with_both_kinds_and_currencies() -> AnalysisSummary {
        let categories = [
            (TransactionKind::Income, Currency::Rub, "Зарплата"),
            (TransactionKind::Income, Currency::Usd, "Доход USD"),
            (TransactionKind::Expense, Currency::Rub, "Покупки"),
            (TransactionKind::Expense, Currency::Usd, "Расход USD"),
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

    fn render_summary_at_width(
        summary: &AnalysisSummary,
        mode: AnalysisMode,
        width: f32,
    ) -> egui::FullOutput {
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
                        AnalysisPage::render_summary(ui, summary, mode);
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

    #[test]
    fn stale_analysis_result_does_not_replace_current_filter_result() {
        let mut page = AnalysisPage::default();
        let previous_filters = page.filters();
        page.schedule(previous_filters.clone(), std::time::Instant::now());
        let previous_generation = page.generation;
        let current_filters = AnalysisFilters {
            from: "2027-01-01".to_owned(),
            to: String::new(),
            mode: AnalysisMode::All,
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
    fn analysis_filter_defaults_to_all_operation_types() {
        assert_eq!(AnalysisPage::default().filters().mode, AnalysisMode::All);
    }

    #[test]
    fn analysis_filter_rejects_reversed_period_boundaries() {
        let filters = AnalysisFilters {
            from: "2027-02-01".to_owned(),
            to: "2027-01-31".to_owned(),
            mode: AnalysisMode::All,
        };

        assert_eq!(
            parse_filters(&filters),
            Err("Начало периода не может быть позже его окончания".to_owned())
        );
    }

    #[test]
    fn all_analysis_mode_renders_income_and_expenses_in_separate_columns() {
        let summary = summary_with_both_kinds_and_currencies();
        let output = render_summary_at_width(&summary, AnalysisMode::All, 1_280.0);

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
    fn single_kind_analysis_mode_renders_only_the_selected_group() {
        let summary = summary_with_both_kinds_and_currencies();
        for (mode, selected_heading, hidden_heading, selected_category, hidden_category) in [
            (
                AnalysisMode::Income,
                "Доходы — RUB",
                "Расходы — RUB",
                "Зарплата",
                "Покупки",
            ),
            (
                AnalysisMode::Expenses,
                "Расходы — RUB",
                "Доходы — RUB",
                "Покупки",
                "Зарплата",
            ),
        ] {
            let output = render_summary_at_width(&summary, mode, 1_280.0);

            assert!(has_text(&output, selected_heading));
            assert!(!has_text(&output, hidden_heading));
            assert!(has_text(&output, selected_category));
            assert!(!has_text(&output, hidden_category));
        }
    }

    #[test]
    fn all_analysis_mode_shows_empty_group_message_when_only_one_kind_has_data() {
        let mut summary = summary_with_both_kinds_and_currencies();
        summary
            .categories
            .retain(|category| category.kind == TransactionKind::Income);
        summary
            .totals
            .retain(|total| total.kind == TransactionKind::Income);
        let output = render_summary_at_width(&summary, AnalysisMode::All, 1_280.0);

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
        let output = render_summary_at_width(&summary, AnalysisMode::All, 1_280.0);

        assert!(has_text(&output, "За выбранный период операций нет."));
    }
}
