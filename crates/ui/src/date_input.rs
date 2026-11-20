use std::hash::Hash;

use chrono::{Datelike, Local, Months, NaiveDate};
use eframe::egui;

const DATE_FORMAT: &str = "%Y-%m-%d";
const MONTH_NAMES: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];
const WEEKDAY_NAMES: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

#[derive(Default)]
pub(crate) struct DateInputState {
    visible_month: Option<NaiveDate>,
    observed_value: Option<String>,
}

impl DateInputState {
    pub(crate) fn reset(&mut self) {
        self.visible_month = None;
        self.observed_value = None;
    }

    fn sync_to_value(&mut self, value: &str, today: NaiveDate) {
        if self.observed_value.as_deref() != Some(value) {
            self.visible_month = Some(initial_month(value, today));
            self.observed_value = Some(value.to_owned());
        }
    }

    fn select_date(&mut self, date: NaiveDate) -> String {
        let value = date.format(DATE_FORMAT).to_string();
        self.visible_month = Some(month_start(date));
        self.observed_value = Some(value.clone());
        value
    }

    fn select_month(&mut self, month: u32) {
        let Some(visible_month) = self.visible_month else {
            return;
        };
        if let Some(selected_month) = NaiveDate::from_ymd_opt(visible_month.year(), month, 1) {
            self.visible_month = Some(selected_month);
        }
    }

    fn select_year(&mut self, year: i32) {
        let Some(visible_month) = self.visible_month else {
            return;
        };
        if let Some(selected_month) = NaiveDate::from_ymd_opt(year, visible_month.month(), 1) {
            self.visible_month = Some(selected_month);
        }
    }
}

pub(crate) fn date_input(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut String,
    state: &mut DateInputState,
    width: f32,
) {
    state.sync_to_value(value, Local::now().date_naive());

    ui.push_id(id_source, |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .desired_width(width)
                    .hint_text("ГГГГ-ММ-ДД"),
            );
            ui.menu_button("📅", |ui| show_calendar(ui, value, state))
                .response
                .on_hover_text("Выбрать дату");
        });
    });
}

fn show_calendar(ui: &mut egui::Ui, value: &mut String, state: &mut DateInputState) {
    ui.set_min_width(224.0);
    ui.horizontal(|ui| {
        let previous = state
            .visible_month
            .and_then(|month| month.checked_sub_months(Months::new(1)));
        if ui
            .add_enabled(previous.is_some(), egui::Button::new("‹"))
            .clicked()
        {
            state.visible_month = previous;
        }

        if let Some(month) = state.visible_month {
            let mut selected_month = month.month();
            egui::ComboBox::from_id_salt("calendar_month")
                .selected_text(MONTH_NAMES[month.month0() as usize])
                .show_ui(ui, |ui| {
                    for (index, name) in MONTH_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut selected_month, index as u32 + 1, *name);
                    }
                });
            if selected_month != month.month() {
                state.select_month(selected_month);
            }

            let mut selected_year = month.year();
            let (min_year, max_year) = year_bounds(month.month());
            ui.add(
                egui::DragValue::new(&mut selected_year)
                    .range(min_year..=max_year)
                    .speed(1),
            );
            if selected_year != month.year() {
                state.select_year(selected_year);
            }
        }

        let next = state
            .visible_month
            .and_then(|month| month.checked_add_months(Months::new(1)));
        if ui
            .add_enabled(next.is_some(), egui::Button::new("›"))
            .clicked()
        {
            state.visible_month = next;
        }
    });

    ui.separator();
    let Some(month) = state.visible_month else {
        return;
    };
    let days = month_grid(month);
    let selected_date = NaiveDate::parse_from_str(value.trim(), DATE_FORMAT).ok();

    egui::Grid::new("calendar_days")
        .num_columns(7)
        .show(ui, |ui| {
            for weekday in WEEKDAY_NAMES {
                ui.strong(weekday);
            }
            ui.end_row();

            for week in 0..6 {
                for weekday in 0..7 {
                    if let Some(date) = days[week * 7 + weekday] {
                        let label = date.day().to_string();
                        if ui
                            .add_sized(
                                [28.0, 26.0],
                                egui::Button::new(label).selected(selected_date == Some(date)),
                            )
                            .clicked()
                        {
                            *value = state.select_date(date);
                            ui.close_menu();
                        }
                    } else {
                        ui.add_sized([28.0, 26.0], egui::Label::new(" "));
                    }
                }
                ui.end_row();
            }
        });
}

fn initial_month(value: &str, today: NaiveDate) -> NaiveDate {
    match NaiveDate::parse_from_str(value.trim(), DATE_FORMAT) {
        Ok(date) => month_start(date),
        Err(_) => month_start(today),
    }
}

fn month_start(date: NaiveDate) -> NaiveDate {
    match NaiveDate::from_ymd_opt(date.year(), date.month(), 1) {
        Some(month_start) => month_start,
        None => date,
    }
}

fn year_bounds(month: u32) -> (i32, i32) {
    let min_year = NaiveDate::MIN.year();
    let max_year = NaiveDate::MAX.year();
    let min_year = if NaiveDate::from_ymd_opt(min_year, month, 1).is_some() {
        min_year
    } else {
        min_year + 1
    };
    let max_year = if NaiveDate::from_ymd_opt(max_year, month, 1).is_some() {
        max_year
    } else {
        max_year - 1
    };

    (min_year, max_year)
}

fn month_grid(month: NaiveDate) -> [Option<NaiveDate>; 42] {
    let first_day = month_start(month);
    let first_weekday = first_day.weekday().num_days_from_monday() as usize;
    let days_in_month = match first_day.month() {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if NaiveDate::from_ymd_opt(first_day.year(), 2, 29).is_some() => 29,
        2 => 28,
        _ => 0,
    };

    std::array::from_fn(|index| {
        let day = index as i64 - first_weekday as i64 + 1;
        if (1..=days_in_month).contains(&day) {
            NaiveDate::from_ymd_opt(first_day.year(), first_day.month(), day as u32)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{initial_month, month_grid, month_start, year_bounds, DateInputState};
    use chrono::{Datelike, NaiveDate};

    #[test]
    fn month_grid_places_leap_day_in_the_correct_weekday_column() {
        let month = NaiveDate::from_ymd_opt(2024, 2, 1).expect("valid month");

        let grid = month_grid(month);

        assert_eq!(grid[31], NaiveDate::from_ymd_opt(2024, 2, 29));
    }

    #[test]
    fn month_grid_leaves_dates_outside_a_non_leap_february_empty() {
        let month = NaiveDate::from_ymd_opt(2023, 2, 1).expect("valid month");

        let grid = month_grid(month);

        assert_eq!(grid.iter().flatten().count(), 28);
    }

    #[test]
    fn month_grid_starts_on_monday_and_offsets_a_sunday_first_month() {
        let month = NaiveDate::from_ymd_opt(2026, 11, 1).expect("valid month");

        let grid = month_grid(month);

        assert_eq!(grid[6], Some(month));
    }

    #[test]
    fn initial_month_uses_the_entered_date_when_valid() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).expect("valid date");

        let month = initial_month("2024-12-25", today);

        assert_eq!(
            month,
            NaiveDate::from_ymd_opt(2024, 12, 1).expect("valid month")
        );
    }

    #[test]
    fn initial_month_uses_today_for_empty_or_invalid_text() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).expect("valid date");
        let expected = NaiveDate::from_ymd_opt(2026, 11, 1).expect("valid month");

        assert_eq!(initial_month("", today), expected);
        assert_eq!(initial_month("not a date", today), expected);
    }

    #[test]
    fn selecting_a_date_formats_it_and_moves_the_calendar_to_its_month() {
        let selected = NaiveDate::from_ymd_opt(2024, 2, 29).expect("valid date");
        let mut state = DateInputState::default();

        let value = state.select_date(selected);

        assert_eq!(value, "2024-02-29");
        assert_eq!(state.visible_month, Some(month_start(selected)));
    }

    #[test]
    fn changing_manual_text_resets_the_calendar_month() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).expect("valid date");
        let changed_date = NaiveDate::from_ymd_opt(2025, 3, 17).expect("valid date");
        let mut state = DateInputState::default();
        state.sync_to_value("2024-12-25", today);

        state.sync_to_value("2025-03-17", today);

        assert_eq!(state.visible_month, Some(month_start(changed_date)));
    }

    #[test]
    fn reset_clears_the_calendar_month_and_observed_date() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).expect("valid date");
        let mut state = DateInputState::default();
        state.sync_to_value("2024-12-25", today);

        state.reset();

        assert!(state.visible_month.is_none());
        assert!(state.observed_value.is_none());
    }

    #[test]
    fn checked_month_navigation_crosses_year_boundaries() {
        let december = NaiveDate::from_ymd_opt(2024, 12, 1).expect("valid month");
        let january = NaiveDate::from_ymd_opt(2025, 1, 1).expect("valid month");

        assert_eq!(
            december.checked_add_months(chrono::Months::new(1)),
            Some(january)
        );
        assert_eq!(
            january.checked_sub_months(chrono::Months::new(1)),
            Some(december)
        );
    }

    #[test]
    fn selecting_month_and_year_changes_calendar_without_changing_date_value() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).expect("valid date");
        let mut state = DateInputState::default();
        state.sync_to_value("2024-01-15", today);

        state.select_month(2);
        state.select_year(2023);

        assert_eq!(
            state.visible_month,
            Some(NaiveDate::from_ymd_opt(2023, 2, 1).expect("valid month"))
        );
        assert_eq!(state.observed_value.as_deref(), Some("2024-01-15"));
    }

    #[test]
    fn selecting_year_updates_the_leap_day_grid() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).expect("valid date");
        let mut state = DateInputState::default();
        state.sync_to_value("2024-01-15", today);
        state.select_month(2);

        let leap_february = month_grid(state.visible_month.expect("visible month"));
        assert_eq!(leap_february.iter().flatten().count(), 29);

        state.select_year(2023);

        let common_february = month_grid(state.visible_month.expect("visible month"));
        assert_eq!(common_february.iter().flatten().count(), 28);
    }

    #[test]
    fn year_bounds_include_only_years_with_valid_months_in_chrono_range() {
        for month in 1..=12 {
            let (min_year, max_year) = year_bounds(month);

            assert!(min_year >= NaiveDate::MIN.year());
            assert!(max_year <= NaiveDate::MAX.year());
            assert!(NaiveDate::from_ymd_opt(min_year, month, 1).is_some());
            assert!(NaiveDate::from_ymd_opt(max_year, month, 1).is_some());
            assert!(NaiveDate::from_ymd_opt(min_year - 1, month, 1).is_none());
            assert!(NaiveDate::from_ymd_opt(max_year + 1, month, 1).is_none());
        }
    }
}
