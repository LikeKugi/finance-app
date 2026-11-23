//! Чистые функции-хелперы для универсальной логики отображения.

use chrono::NaiveDate;

/// Единый формат отображения дат: `DD.MM.YYYY`.
pub(crate) fn format_date(date: NaiveDate) -> String {
    date.format("%d.%m.%Y").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_date_returns_dd_mm_yyyy() {
        let date = NaiveDate::from_ymd_opt(2024, 5, 3).unwrap();
        assert_eq!(format_date(date), "03.05.2024");
    }

    #[test]
    fn format_date_handles_leap_year() {
        let date = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        assert_eq!(format_date(date), "29.02.2024");
    }

    #[test]
    fn format_date_first_of_month() {
        let date = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
        assert_eq!(format_date(date), "01.01.2023");
    }
}
