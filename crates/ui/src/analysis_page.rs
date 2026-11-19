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

        egui::ScrollArea::vertical().show(ui, |ui| {
            for kind in [TransactionKind::Income, TransactionKind::Expense] {
                if mode
                    .transaction_kind()
                    .is_some_and(|selected_kind| selected_kind != kind)
                {
                    continue;
                }

                for currency in Currency::ALL {
                    let Some(total) = summary
                        .totals
                        .iter()
                        .find(|total| total.kind == kind && total.currency == currency)
                    else {
                        continue;
                    };

                    ui.heading(format!("{} — {}", kind.title(), currency.code()));
                    egui::Grid::new(format!("analysis_{kind:?}_{}", currency.code()))
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Категория");
                            ui.strong("Операций");
                            ui.strong("Сумма");
                            ui.strong("Доля");
                            ui.end_row();

                            for category in summary.categories.iter().filter(|category| {
                                category.kind == kind && category.currency == currency
                            }) {
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
            }
        });
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
    use finance_core::AnalysisSummary;

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
}
