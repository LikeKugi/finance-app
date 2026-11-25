//! Общие чистые функции интерфейса, не зависящие от GUI-фреймворка.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Days, Months, NaiveDate};

use super::helpers::format_date;
use finance_core::{Category, Currency, CurrencyDefinition, PaymentDocument};
use rust_decimal::RoundingStrategy;

pub(crate) const DIVISION_ROUNDING_WARNING: &str = "Деление округляется до 2 знаков после запятой; погрешность может накапливаться при нескольких делениях.";
const MAX_COMMENT_SUGGESTIONS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DatePeriod {
    All,
    Year,
    Quarter,
    Month,
    Week,
    Day,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DatePeriodRange {
    Unbounded,
    Bounded(NaiveDate, NaiveDate),
}

impl DatePeriod {
    pub(crate) const ALL: [(Self, &'static str); 6] = [
        (Self::All, "Весь период"),
        (Self::Year, "Год"),
        (Self::Quarter, "Квартал"),
        (Self::Month, "Месяц"),
        (Self::Week, "Неделя"),
        (Self::Day, "День"),
    ];
}

pub(crate) fn date_period_range(period: DatePeriod, today: NaiveDate) -> Option<DatePeriodRange> {
    match period {
        DatePeriod::All => Some(DatePeriodRange::Unbounded),
        DatePeriod::Year => Some(DatePeriodRange::Bounded(
            NaiveDate::from_ymd_opt(today.year(), 1, 1)?,
            NaiveDate::from_ymd_opt(today.year(), 12, 31)?,
        )),
        DatePeriod::Quarter => {
            let first_month = (today.month0() / 3) * 3 + 1;
            let from = NaiveDate::from_ymd_opt(today.year(), first_month, 1)?;
            let to = from.checked_add_months(Months::new(3))?.pred_opt()?;
            Some(DatePeriodRange::Bounded(from, to))
        }
        DatePeriod::Month => {
            let from = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?;
            let to = from.checked_add_months(Months::new(1))?.pred_opt()?;
            Some(DatePeriodRange::Bounded(from, to))
        }
        DatePeriod::Week => {
            let from = today
                .checked_sub_days(Days::new(u64::from(today.weekday().num_days_from_monday())))?;
            let to = from.checked_add_days(Days::new(6))?;
            Some(DatePeriodRange::Bounded(from, to))
        }
        DatePeriod::Day => Some(DatePeriodRange::Bounded(today, today)),
    }
}

#[cfg(test)]
pub(crate) fn set_period_dates(
    period: DatePeriod,
    today: NaiveDate,
    from_date: &mut String,
    to_date: &mut String,
) -> bool {
    let Some(range) = date_period_range(period, today) else {
        return false;
    };
    match range {
        DatePeriodRange::Unbounded => {
            from_date.clear();
            to_date.clear();
        }
        DatePeriodRange::Bounded(from, to) => {
            *from_date = from.format(super::helpers::DATE_FORMAT).to_string();
            *to_date = to.format(super::helpers::DATE_FORMAT).to_string();
        }
    }
    true
}

pub(crate) fn text_suggestions(history: &[String], query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    history
        .iter()
        .filter_map(|comment| {
            let normalized = comment.to_lowercase();
            (normalized != query && normalized.contains(&query) && seen.insert(normalized))
                .then(|| comment.clone())
        })
        .take(MAX_COMMENT_SUGGESTIONS)
        .collect()
}

pub(crate) fn currency_rate_label(currency: &CurrencyDefinition, primary_code: &str) -> String {
    if currency.is_primary {
        return "Основная валюта".to_owned();
    }
    match (currency.rate_to_primary, currency.rate_date) {
        (Some(rate), Some(date)) => {
            let rounded_rate =
                rate.round_dp_with_strategy(5, RoundingStrategy::MidpointNearestEven);
            let rate_text = if !rate.is_zero() && rounded_rate.is_zero() {
                if rate.is_sign_negative() {
                    ">-0.00001".to_owned()
                } else {
                    "<0.00001".to_owned()
                }
            } else {
                let formatted_rate = format!("{rounded_rate:.5}");
                if rate == rounded_rate {
                    formatted_rate
                } else {
                    format!("{formatted_rate} (округлено)")
                }
            };
            format!(
                "1 {} = {} {} · курс от {}",
                currency.code,
                rate_text,
                primary_code,
                format_date(date)
            )
        }
        _ => "Курс не задан".to_owned(),
    }
}

pub(crate) fn category_tree_rows(categories: &[Category]) -> Vec<(usize, usize)> {
    let mut category_ids = HashSet::with_capacity(categories.len());
    let mut children_by_parent = HashMap::<i64, Vec<usize>>::new();
    let mut roots = Vec::new();

    for category in categories {
        category_ids.insert(category.id);
    }
    for (index, category) in categories.iter().enumerate() {
        match category.parent_id {
            Some(parent_id) if category_ids.contains(&parent_id) => {
                children_by_parent.entry(parent_id).or_default().push(index);
            }
            _ => roots.push(index),
        }
    }

    let mut rows = Vec::with_capacity(categories.len());
    let mut visited = vec![false; categories.len()];
    for root in roots.into_iter().chain(0..categories.len()) {
        if visited[root] {
            continue;
        }

        let mut pending = vec![(root, 0)];
        while let Some((index, depth)) = pending.pop() {
            if visited[index] {
                continue;
            }

            visited[index] = true;
            rows.push((index, depth));
            if let Some(children) = children_by_parent.get(&categories[index].id) {
                pending.extend(children.iter().rev().map(|&child| (child, depth + 1)));
            }
        }
    }

    rows
}

pub(crate) fn category_tree_labels(categories: &[Category]) -> Vec<(i64, String)> {
    let mut path = Vec::new();

    category_tree_rows(categories)
        .into_iter()
        .map(|(index, depth)| {
            path.truncate(depth);
            path.push(categories[index].name.as_str());
            (categories[index].id, path.join(" > "))
        })
        .collect()
}

/// Собирает ID указанной категории и всех её вложенных подкатегорий рекурсивно.
pub(crate) fn collect_category_descendants(categories: &[Category], root_id: i64) -> Vec<i64> {
    let mut children_by_parent = HashMap::<i64, Vec<i64>>::new();
    for category in categories {
        if let Some(parent_id) = category.parent_id {
            children_by_parent
                .entry(parent_id)
                .or_default()
                .push(category.id);
        }
    }

    let mut result = Vec::new();
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        result.push(id);
        if let Some(children) = children_by_parent.get(&id) {
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
    }
    result
}

/// Карты, доступные для выбора при данной валюте операции.
pub(crate) fn eligible_cards(
    cards: &[PaymentDocument],
    currency: Option<Currency>,
) -> Vec<&PaymentDocument> {
    cards
        .iter()
        .filter(|card| card.currency.is_some() && card.currency == currency)
        .collect()
}

/// Пояснение к полю суммы компактной формы: дата записи без поля выбора даты.
pub(crate) fn quick_form_date_caption(date: NaiveDate) -> String {
    format!("Дата: сегодня ({})", format_date(date))
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_core::PaymentCardType;
    use rust_decimal::Decimal;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid test date")
    }

    fn payment_document(id: i64, currency: Option<Currency>) -> PaymentDocument {
        PaymentDocument {
            id,
            name: None,
            bank_id: 1,
            bank_name: "Альфа-Банк".to_owned(),
            payment_system: finance_core::PaymentSystem::Mir,
            card_type: PaymentCardType::Debit,
            amount: Decimal::ONE,
            currency,
            due_date: None,
            grace_period_days: None,
            next_payment_amount: None,
            affects_deposit_rate: false,
            min_monthly_spend: None,
            linked_savings: Vec::new(),
        }
    }

    #[test]
    fn quick_form_date_caption_shows_the_actual_date() {
        assert_eq!(
            quick_form_date_caption(date(2026, 11, 21)),
            "Дата: сегодня (21.11.2026)"
        );
    }

    #[test]
    fn eligible_cards_keep_only_the_matching_currency() {
        let rub = payment_document(1, Some(Currency::Rub));
        let usd = payment_document(2, Some(Currency::Usd));
        let unknown = payment_document(3, None);
        let cards = [rub, usd, unknown];

        let ids: Vec<i64> = eligible_cards(&cards, Some(Currency::Rub))
            .into_iter()
            .map(|card| card.id)
            .collect();
        assert_eq!(ids, vec![1]);
        assert!(eligible_cards(&cards, None).is_empty());
    }

    #[test]
    fn year_period_uses_full_calendar_year() {
        assert_eq!(
            date_period_range(DatePeriod::Year, date(2024, 2, 29)),
            Some(DatePeriodRange::Bounded(
                date(2024, 1, 1),
                date(2024, 12, 31)
            ))
        );
    }

    #[test]
    fn quarter_period_uses_current_calendar_quarter() {
        assert_eq!(
            date_period_range(DatePeriod::Quarter, date(2026, 11, 20)),
            Some(DatePeriodRange::Bounded(
                date(2026, 10, 1),
                date(2026, 12, 31)
            ))
        );
    }

    #[test]
    fn month_period_includes_leap_day() {
        assert_eq!(
            date_period_range(DatePeriod::Month, date(2024, 2, 29)),
            Some(DatePeriodRange::Bounded(
                date(2024, 2, 1),
                date(2024, 2, 29)
            ))
        );
    }

    #[test]
    fn week_period_starts_on_monday_across_year_boundary() {
        assert_eq!(
            date_period_range(DatePeriod::Week, date(2026, 1, 1)),
            Some(DatePeriodRange::Bounded(
                date(2025, 12, 29),
                date(2026, 1, 4)
            ))
        );
    }

    #[test]
    fn day_period_uses_same_start_and_end_date() {
        let today = date(2026, 11, 20);

        assert_eq!(
            date_period_range(DatePeriod::Day, today),
            Some(DatePeriodRange::Bounded(today, today))
        );
    }

    #[test]
    fn all_period_is_the_default_period_option() {
        assert_eq!(DatePeriod::ALL[0], (DatePeriod::All, "Весь период"));
        assert_eq!(
            date_period_range(DatePeriod::All, date(2024, 2, 29)),
            Some(DatePeriodRange::Unbounded)
        );
    }

    #[test]
    fn selecting_all_period_clears_both_date_inputs() {
        let today = date(2024, 2, 29);
        let mut from = "не дата".to_owned();
        let mut to = "01.03.2024".to_owned();

        assert!(set_period_dates(DatePeriod::All, today, &mut from, &mut to));

        assert!(from.is_empty());
        assert!(to.is_empty());
    }

    #[test]
    fn selecting_bounded_period_after_all_replaces_both_dates() {
        let today = date(2024, 2, 29);
        let mut from = String::new();
        let mut to = String::new();

        assert!(set_period_dates(
            DatePeriod::Year,
            today,
            &mut from,
            &mut to
        ));
        assert_eq!(from, "01.01.2024");
        assert_eq!(to, "31.12.2024");

        assert!(set_period_dates(DatePeriod::Day, today, &mut from, &mut to));
        assert_eq!(from, "29.02.2024");
        assert_eq!(to, "29.02.2024");

        from = "28.02.2024".to_owned();
        to = "01.03.2024".to_owned();
        assert_eq!(from, "28.02.2024");
        assert_eq!(to, "01.03.2024");
    }

    #[test]
    fn failed_period_range_keeps_both_date_inputs_unchanged() {
        let mut from = "начало".to_owned();
        let mut to = "конец".to_owned();

        assert!(!set_period_dates(
            DatePeriod::Quarter,
            NaiveDate::MAX,
            &mut from,
            &mut to
        ));

        assert_eq!(from, "начало");
        assert_eq!(to, "конец");
    }

    fn category(id: i64, parent_id: Option<i64>, is_active: bool) -> Category {
        Category {
            id,
            name: format!("Категория {id}"),
            parent_id,
            is_active,
            is_regular: None,
        }
    }

    #[test]
    fn category_tree_rows_nest_children_and_keep_sibling_order() {
        let categories = vec![
            category(3, Some(2), true),
            category(20, None, true),
            category(4, Some(3), true),
            category(2, None, true),
            category(5, Some(2), true),
        ];

        let rows: Vec<_> = category_tree_rows(&categories)
            .into_iter()
            .map(|(index, depth)| (categories[index].id, depth))
            .collect();

        assert_eq!(rows, vec![(20, 0), (2, 0), (3, 1), (4, 2), (5, 1)]);
    }

    #[test]
    fn category_tree_rows_keep_inactive_categories_visible() {
        let categories = vec![category(1, None, true), category(2, Some(1), false)];

        let rows: Vec<_> = category_tree_rows(&categories)
            .into_iter()
            .map(|(index, depth)| (categories[index].id, depth, categories[index].is_active))
            .collect();

        assert_eq!(rows, vec![(1, 0, true), (2, 1, false)]);
    }

    #[test]
    fn category_tree_labels_show_paths_and_keep_inactive_categories() {
        let mut categories = vec![
            category(3, Some(2), true),
            category(20, None, true),
            category(4, Some(3), false),
            category(2, None, false),
            category(5, Some(2), true),
            category(6, Some(20), true),
        ];
        categories[0].name = "Разное".to_owned();
        categories[1].name = "Транспорт".to_owned();
        categories[2].name = "История".to_owned();
        categories[3].name = "Расходы".to_owned();
        categories[4].name = "Разное".to_owned();
        categories[5].name = "Разное".to_owned();

        let rows = category_tree_labels(&categories);

        assert_eq!(
            rows,
            vec![
                (20, "Транспорт".to_owned()),
                (6, "Транспорт > Разное".to_owned()),
                (2, "Расходы".to_owned()),
                (3, "Расходы > Разное".to_owned()),
                (4, "Расходы > Разное > История".to_owned()),
                (5, "Расходы > Разное".to_owned()),
            ]
        );
    }

    #[test]
    fn category_tree_rows_include_missing_parents_and_cycles() {
        let categories = vec![
            category(1, Some(2), true),
            category(2, Some(1), true),
            category(3, Some(99), true),
            category(4, Some(3), true),
            category(5, None, true),
        ];

        let rows: Vec<_> = category_tree_rows(&categories)
            .into_iter()
            .map(|(index, depth)| (categories[index].id, depth))
            .collect();

        assert_eq!(rows, vec![(3, 0), (4, 1), (5, 0), (1, 0), (2, 1)]);
    }

    #[test]
    fn text_suggestions_match_case_insensitive_substrings_and_limit_unique_results() {
        let history = [
            "Покупки в магазине",
            "Покупки в МАГАЗИНЕ",
            "магазин у дома",
            "В магазин",
            "Магазин за углом",
            "Ещё один магазин",
            "Последний магазин",
        ]
        .map(str::to_owned);

        assert_eq!(
            text_suggestions(&history, "МАГАЗИН"),
            [
                "Покупки в магазине",
                "магазин у дома",
                "В магазин",
                "Магазин за углом",
                "Ещё один магазин",
            ]
        );
        assert!(text_suggestions(&history, "  ").is_empty());
        assert!(text_suggestions(&history, "покупки в магазине").is_empty());
    }

    #[test]
    fn currency_rate_label_does_not_hide_nonzero_rates_below_display_precision() {
        let currency = CurrencyDefinition {
            code: "BTC".to_owned(),
            name: "Биткоин".to_owned(),
            is_primary: false,
            rate_to_primary: Some(Decimal::new(1, 7)),
            rate_date: Some(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()),
        };

        assert_eq!(
            currency_rate_label(&currency, "RUB"),
            "1 BTC = <0.00001 RUB · курс от 01.02.2026"
        );
    }

    #[test]
    fn collect_category_descendants_includes_self_and_all_descendants() {
        let categories = vec![
            category(1, None, true),    // root
            category(2, Some(1), true), // child of 1
            category(3, Some(1), true), // child of 1
            category(4, Some(2), true), // grandchild of 1
            category(5, Some(4), true), // great-grandchild of 1
            category(10, None, true),   // unrelated root
        ];

        // Root category includes all descendants
        let mut ids = collect_category_descendants(&categories, 1);
        ids.sort();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);

        // Subcategory includes only its subtree
        let mut ids = collect_category_descendants(&categories, 2);
        ids.sort();
        assert_eq!(ids, vec![2, 4, 5]);

        // Leaf category includes only itself
        let ids = collect_category_descendants(&categories, 3);
        assert_eq!(ids, vec![3]);

        // Unrelated category
        let ids = collect_category_descendants(&categories, 10);
        assert_eq!(ids, vec![10]);
    }
}
