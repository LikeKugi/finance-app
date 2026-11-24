//! Чистые функции-хелперы для универсальной логики отображения.

use chrono::NaiveDate;
use finance_core::{Currency, PaymentCardType, PaymentDocument};
use rust_decimal::Decimal;

/// Единый формат отображения дат: `DD.MM.YYYY`.
pub(crate) const DATE_FORMAT: &str = "%d.%m.%Y";

/// Форматирует дату в едином формате `DD.MM.YYYY`.
pub(crate) fn format_date(date: NaiveDate) -> String {
    date.format(DATE_FORMAT).to_string()
}

/// Форматирует денежную сумму с разделителями разрядов.
///
/// Возвращает строку вида `DDD DDD ... DDD.DD`, где количество десятичных
/// знаков определяется параметром `fraction_digits` (по умолчанию `2`).
///
/// # Примеры
///
/// ```ignore
/// # use rust_decimal::Decimal;
/// # use std::str::FromStr;
/// # use crate::gpui::helpers::format_currency;
/// let amount = Decimal::from(1234567u32);
/// assert_eq!(format_currency(&amount, 2), "1 234 567.00");
///
/// let amount = Decimal::from_str("12345.678").unwrap();
/// assert_eq!(format_currency(&amount, 3), "12 345.678");
/// ```
pub(crate) fn format_currency(amount: &Decimal, fraction_digits: u32) -> String {
    // Формируем строковое представление с нужным числом десятичных знаков.
    // rust_decimal::to_string() выводит каноническое значение с фактическим scale,
    // поэтому вручную разделяем целую и дробную части и дополняем/обрезаем дробную часть.
    let negative = amount.is_sign_negative();

    // Получаем абсолютное значение для работы с цифрами
    let abs = amount.abs();
    let s = abs.to_string();

    let (integer_part, mut fractional_part) = if let Some(dot_pos) = s.find('.') {
        (&s[..dot_pos], s[dot_pos + 1..].to_string())
    } else {
        (s.as_str(), String::new())
    };

    // Дополняем или обрезаем дробную часть до нужного количества знаков
    let frac_len = fraction_digits as usize;
    if fractional_part.len() < frac_len {
        fractional_part.push_str(&"0".repeat(frac_len - fractional_part.len()));
    } else if fractional_part.len() > frac_len {
        fractional_part.truncate(frac_len);
    }

    // Группируем целую часть по 3 цифры справа налево
    let bytes = integer_part.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len + frac_len + len / 3 + 1);

    if negative {
        result.push('-');
    }

    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(' ');
        }
        result.push(b as char);
    }

    if !fractional_part.is_empty() {
        result.push('.');
        result.push_str(&fractional_part);
    }

    result
}

const UNKNOWN_CARD_CURRENCY_LABEL: &str = "валюта не указана";

/// Единая подпись карты для опций выбора: `Название - Банк - Система (Тип; Валюта)`.
///
/// Карта без названия подписывается как `Банк - Система (Тип; Валюта)`.
pub(crate) fn card_label(card: &PaymentDocument) -> String {
    let card_type = match card.card_type {
        PaymentCardType::Debit => "дебетовая",
        PaymentCardType::Credit => "кредитная",
    };
    let details = format!(
        "{} - {} ({card_type}; {})",
        card.bank_name,
        card.payment_system.code(),
        card.currency
            .map_or(UNKNOWN_CARD_CURRENCY_LABEL, Currency::code)
    );
    match card
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(name) => format!("{name} - {details}"),
        None => details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_core::PaymentSystem;
    use std::str::FromStr;

    fn payment_document(
        id: i64,
        name: Option<&str>,
        card_type: PaymentCardType,
        currency: Option<Currency>,
    ) -> PaymentDocument {
        PaymentDocument {
            id,
            name: name.map(str::to_owned),
            bank_id: 1,
            bank_name: "Альфа-Банк".to_owned(),
            payment_system: PaymentSystem::Mir,
            card_type,
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

    #[test]
    fn format_currency_groups_by_thousands() {
        let amount = Decimal::from(1234567u32);
        assert_eq!(format_currency(&amount, 2), "1 234 567.00");
    }

    #[test]
    fn format_currency_with_fractional_digits_2() {
        let amount = Decimal::from_str("1234.56").unwrap();
        assert_eq!(format_currency(&amount, 2), "1 234.56");
    }

    #[test]
    fn format_currency_with_fractional_digits_3() {
        let amount = Decimal::from_str("12345.678").unwrap();
        assert_eq!(format_currency(&amount, 3), "12 345.678");
    }

    #[test]
    fn format_currency_with_fractional_digits_5() {
        let amount = Decimal::from_str("99.12345").unwrap();
        assert_eq!(format_currency(&amount, 5), "99.12345");
    }

    #[test]
    fn format_currency_zero_amount() {
        let amount = Decimal::ZERO;
        assert_eq!(format_currency(&amount, 2), "0.00");
    }

    #[test]
    fn format_currency_single_digit() {
        let amount = Decimal::from(5u32);
        assert_eq!(format_currency(&amount, 2), "5.00");
    }

    #[test]
    fn format_currency_three_digits_no_separator() {
        let amount = Decimal::from(999u32);
        assert_eq!(format_currency(&amount, 2), "999.00");
    }

    #[test]
    fn format_currency_exactly_four_digits() {
        let amount = Decimal::from(1000u32);
        assert_eq!(format_currency(&amount, 2), "1 000.00");
    }

    #[test]
    fn format_currency_large_amount() {
        let amount = Decimal::from_str("1234567890.12").unwrap();
        assert_eq!(format_currency(&amount, 2), "1 234 567 890.12");
    }

    #[test]
    fn format_currency_negative_amount() {
        let amount = Decimal::from_str("-1234.56").unwrap();
        assert_eq!(format_currency(&amount, 2), "-1 234.56");
    }

    #[test]
    fn format_currency_rounds_up() {
        let amount = Decimal::from_str("100.123").unwrap();
        assert_eq!(format_currency(&amount, 2), "100.12");
    }

    #[test]
    fn card_label_uses_unified_format() {
        let card = payment_document(
            7,
            Some("Повседневная"),
            PaymentCardType::Debit,
            Some(Currency::Rub),
        );

        assert_eq!(
            card_label(&card),
            "Повседневная - Альфа-Банк - MIR (дебетовая; RUB)"
        );
    }

    #[test]
    fn card_label_without_name_starts_with_bank() {
        let card = payment_document(8, None, PaymentCardType::Credit, Some(Currency::Usd));

        assert_eq!(card_label(&card), "Альфа-Банк - MIR (кредитная; USD)");
    }

    #[test]
    fn card_label_without_currency_uses_placeholder() {
        let card = payment_document(9, None, PaymentCardType::Debit, None);

        assert_eq!(
            card_label(&card),
            "Альфа-Банк - MIR (дебетовая; валюта не указана)"
        );
    }

    #[test]
    fn card_label_ignores_blank_name_and_id() {
        let card = payment_document(42, Some("  "), PaymentCardType::Debit, Some(Currency::Rub));

        assert_eq!(card_label(&card), "Альфа-Банк - MIR (дебетовая; RUB)");
    }
}
