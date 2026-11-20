use chrono::{Datelike, Days, Months, NaiveDate};
use eframe::egui;
use finance_core::{Currency, FinancialTransaction, TransactionBalance};
use rust_decimal::Decimal;

use super::{AnalysisPage, ModuleTab};

pub(super) const TABLE_ROW_HEIGHT: f32 = 48.0;
const FILTER_ACTIONS_MIN_WIDTH: f32 = 250.0;
const FILTER_ITEM_SPACING: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DatePeriod {
    Year,
    Quarter,
    Month,
    Week,
    Day,
}

impl DatePeriod {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Year, "Год"),
        (Self::Quarter, "Квартал"),
        (Self::Month, "Месяц"),
        (Self::Week, "Неделя"),
        (Self::Day, "День"),
    ];
}

pub(super) fn render_period_filter(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
) -> Option<DatePeriod> {
    let mut selected_period = None;
    ui.label("Период");
    egui::ComboBox::from_id_salt(id_source)
        .width(110.0)
        .selected_text("Выбрать")
        .show_ui(ui, |ui| {
            for (period, label) in DatePeriod::ALL {
                if ui.selectable_label(false, label).clicked() {
                    selected_period = Some(period);
                }
            }
        });
    selected_period
}

fn date_period_range(period: DatePeriod, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    match period {
        DatePeriod::Year => Some((
            NaiveDate::from_ymd_opt(today.year(), 1, 1)?,
            NaiveDate::from_ymd_opt(today.year(), 12, 31)?,
        )),
        DatePeriod::Quarter => {
            let first_month = (today.month0() / 3) * 3 + 1;
            let from = NaiveDate::from_ymd_opt(today.year(), first_month, 1)?;
            let to = from.checked_add_months(Months::new(3))?.pred_opt()?;
            Some((from, to))
        }
        DatePeriod::Month => {
            let from = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?;
            let to = from.checked_add_months(Months::new(1))?.pred_opt()?;
            Some((from, to))
        }
        DatePeriod::Week => {
            let from = today
                .checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_monday())))?;
            let to = from.checked_add_days(Days::new(6))?;
            Some((from, to))
        }
        DatePeriod::Day => Some((today, today)),
    }
}

pub(super) fn set_period_dates(
    period: DatePeriod,
    today: NaiveDate,
    from_date: &mut String,
    to_date: &mut String,
) -> bool {
    let Some((from, to)) = date_period_range(period, today) else {
        return false;
    };
    *from_date = from.format("%Y-%m-%d").to_string();
    *to_date = to.format("%Y-%m-%d").to_string();
    true
}

pub(super) fn add_button_text(ui: &egui::Ui, label: &str) -> egui::WidgetText {
    let button_font = ui.style().text_styles[&egui::TextStyle::Button].clone();
    let mut symbol_font = button_font.clone();
    symbol_font.size *= 1.4;

    let mut job = egui::text::LayoutJob::default();
    job.append(
        "+",
        0.0,
        egui::TextFormat {
            font_id: symbol_font,
            ..Default::default()
        },
    );
    job.append(
        &format!(" {label}"),
        0.0,
        egui::TextFormat {
            font_id: button_font,
            ..Default::default()
        },
    );
    job.into()
}

pub(super) fn filter_layout(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(FILTER_ITEM_SPACING, FILTER_ITEM_SPACING);
        ui.horizontal_wrapped(add_contents);
    });
}

pub(super) fn filter_group(
    ui: &mut egui::Ui,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        add_contents,
    );
}

pub(super) fn render_filter_actions(ui: &mut egui::Ui, has_active_filters: bool) -> (bool, bool) {
    let available_width = ui.available_size_before_wrap().x;
    let width = if available_width < FILTER_ACTIONS_MIN_WIDTH {
        ui.max_rect().width()
    } else {
        available_width
    };
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let reset = ui
                .add_enabled(has_active_filters, egui::Button::new("Сбросить фильтры"))
                .clicked();
            let apply = ui.button("Применить").clicked();
            (apply, reset)
        },
    )
    .inner
}

pub(super) fn render_analysis_footer(
    context: &egui::Context,
    active_tab: ModuleTab,
    analysis_page: &AnalysisPage,
) {
    if active_tab == ModuleTab::Analysis {
        egui::TopBottomPanel::bottom("analysis_footer").show(context, |ui| {
            ui.separator();
            analysis_page.render_footer(ui);
        });
    }
}

pub(super) fn transaction_balance_period_label(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> String {
    match (from, to) {
        (None, None) => "За весь период".to_owned(),
        (Some(from), Some(to)) => format!(
            "За период с {} по {}",
            from.format("%d.%m.%Y"),
            to.format("%d.%m.%Y")
        ),
        (Some(from), None) => format!("За период с {}", from.format("%d.%m.%Y")),
        (None, Some(to)) => format!("За период по {}", to.format("%d.%m.%Y")),
    }
}

pub(super) fn render_transaction_balance_footer(
    ui: &mut egui::Ui,
    balances: Option<&[TransactionBalance; 2]>,
    period: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
    primary_currency: Option<&str>,
) {
    match (balances, period) {
        (Some(balances), Some((from, to))) => {
            let Some(primary_currency) = primary_currency else {
                ui.colored_label(
                    egui::Color32::RED,
                    "Сальдо не отображается: не удалось определить основную валюту.",
                );
                return;
            };
            let Some(balance) = balances
                .iter()
                .find(|balance| balance.currency.code() == primary_currency)
            else {
                ui.colored_label(
                    egui::Color32::RED,
                    format!(
                        "Сальдо не отображается: основная валюта {primary_currency} не поддерживается для операций (доступны RUB и USD)."
                    ),
                );
                return;
            };
            let period_label = transaction_balance_period_label(from, to);
            let footer_font_size = egui::TextStyle::Body.resolve(ui.style()).size + 4.0;
            ui.label(
                egui::RichText::new(format!("{period_label}:"))
                    .size(footer_font_size)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!(
                    "Доходы: {} {} − Расходы: {} {} = Сальдо: {} {}",
                    balance.income,
                    balance.currency.code(),
                    balance.expenses,
                    balance.currency.code(),
                    balance.balance,
                    balance.currency.code(),
                ))
                .size(footer_font_size)
                .strong(),
            );
            ui.weak(
                "Сальдо операций не включает начальные остатки и переводы и не подтверждает сумму денег на руках.",
            );
        }
        _ => {
            ui.colored_label(
                egui::Color32::RED,
                "Сальдо не рассчитано: обновите список после устранения ошибки.",
            );
        }
    }
}

pub(super) fn render_transaction_list_total(
    ui: &mut egui::Ui,
    transactions: &[FinancialTransaction],
    primary_currency: Option<&str>,
) -> bool {
    let Some(primary_currency) = primary_currency else {
        ui.label(format!(
            "Записей: {}   Итого недоступно: основная валюта не определена",
            transactions.len()
        ));
        return false;
    };
    let Some(currency) = Currency::ALL
        .into_iter()
        .find(|currency| currency.code() == primary_currency)
    else {
        ui.label(format!(
            "Записей: {}   Итого недоступно: основная валюта {primary_currency} не поддерживается для операций",
            transactions.len()
        ));
        return false;
    };
    let Some(total) = transactions
        .iter()
        .filter(|transaction| transaction.currency == currency)
        .try_fold(Decimal::ZERO, |total, transaction| {
            total.checked_add(transaction.amount)
        })
    else {
        ui.colored_label(
            egui::Color32::RED,
            "Не удалось рассчитать итог: сумма слишком велика",
        );
        return true;
    };

    ui.label(format!(
        "Записей: {}   Итого: {total} {primary_currency}",
        transactions.len()
    ));
    false
}

#[cfg(test)]
mod tests {
    use super::{date_period_range, set_period_dates, DatePeriod};
    use chrono::NaiveDate;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid test date")
    }

    #[test]
    fn year_period_uses_full_calendar_year() {
        assert_eq!(
            date_period_range(DatePeriod::Year, date(2024, 2, 29)),
            Some((date(2024, 1, 1), date(2024, 12, 31)))
        );
    }

    #[test]
    fn quarter_period_uses_current_calendar_quarter() {
        assert_eq!(
            date_period_range(DatePeriod::Quarter, date(2026, 11, 20)),
            Some((date(2026, 10, 1), date(2026, 12, 31)))
        );
    }

    #[test]
    fn month_period_includes_leap_day() {
        assert_eq!(
            date_period_range(DatePeriod::Month, date(2024, 2, 29)),
            Some((date(2024, 2, 1), date(2024, 2, 29)))
        );
    }

    #[test]
    fn week_period_starts_on_monday_across_year_boundary() {
        assert_eq!(
            date_period_range(DatePeriod::Week, date(2026, 1, 1)),
            Some((date(2025, 12, 29), date(2026, 1, 4)))
        );
    }

    #[test]
    fn day_period_uses_same_start_and_end_date() {
        let today = date(2026, 11, 20);

        assert_eq!(
            date_period_range(DatePeriod::Day, today),
            Some((today, today))
        );
    }

    #[test]
    fn selecting_period_replaces_both_dates_and_keeps_them_editable() {
        let today = date(2024, 2, 29);
        let mut from = String::new();
        let mut to = String::new();

        assert!(set_period_dates(
            DatePeriod::Year,
            today,
            &mut from,
            &mut to
        ));
        assert_eq!(from, "2024-01-01");
        assert_eq!(to, "2024-12-31");

        assert!(set_period_dates(DatePeriod::Day, today, &mut from, &mut to));
        assert_eq!(from, "2024-02-29");
        assert_eq!(to, "2024-02-29");

        from = "2024-02-28".to_owned();
        to = "2024-03-01".to_owned();
        assert_eq!(from, "2024-02-28");
        assert_eq!(to, "2024-03-01");
    }
}
