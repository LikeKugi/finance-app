use std::str::FromStr;

use chrono::NaiveDate;
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionKind {
    Income,
    Expense,
}

impl TransactionKind {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Income => "Доходы",
            Self::Expense => "Расходы",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    Rub,
    Usd,
}

impl Currency {
    pub const ALL: [Self; 2] = [Self::Rub, Self::Usd];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Rub => "RUB",
            Self::Usd => "USD",
        }
    }
}

impl FromStr for Currency {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "RUB" => Ok(Self::Rub),
            "USD" => Ok(Self::Usd),
            _ => Err(DomainError::UnknownCurrency(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bank {
    pub id: i64,
    pub name: String,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialTransaction {
    pub id: i64,
    pub date: NaiveDate,
    pub category_id: i64,
    pub category_name: String,
    pub amount: Decimal,
    pub currency: Currency,
    pub comment: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("Сумма должна быть положительным числом")]
    InvalidAmount,
    #[error("Некорректная сумма")]
    InvalidAmountFormat,
    #[error("Сумма слишком велика")]
    AmountOverflow,
    #[error("Неизвестная валюта: {0}")]
    UnknownCurrency(String),
}

/// Parses a positive amount without passing through binary floating point.
///
/// A comma is accepted as the decimal separator only when a dot is absent;
/// grouping separators and scientific notation are rejected.
pub fn parse_amount(input: &str) -> Result<Decimal, DomainError> {
    let value = input.trim();
    let ambiguous_grouping = value.split_once(',').is_some_and(|(whole, fraction)| {
        (1..=3).contains(&whole.len()) && fraction.len() == 3
    });
    if value.is_empty()
        || value.contains('.') && value.contains(',')
        || ambiguous_grouping
        || value.chars().any(|character| {
            !character.is_ascii_digit() && character != '.' && character != ','
        })
        || value.matches('.').count() + value.matches(',').count() > 1
    {
        return Err(DomainError::InvalidAmountFormat);
    }

    let normalized = value.replace(',', ".");
    let amount = Decimal::from_str(&normalized).map_err(|_| DomainError::InvalidAmountFormat)?;
    if amount <= Decimal::ZERO {
        return Err(DomainError::InvalidAmount);
    }
    Ok(amount)
}

/// Parses a sum of positive amounts separated by plus signs.
pub fn parse_amount_expression(input: &str) -> Result<Decimal, DomainError> {
    input.split('+').try_fold(Decimal::ZERO, |total, term| {
        let amount = parse_amount(term)?;
        total
            .checked_add(amount)
            .ok_or(DomainError::AmountOverflow)
    })
}

/// Formats a decimal amount in a locale-independent representation for SQLite.
pub fn canonical_amount(amount: Decimal) -> String {
    amount.normalize().to_string()
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::{canonical_amount, parse_amount, parse_amount_expression, DomainError};

    #[test]
    fn parse_amount_preserves_decimal_precision() {
        let amount = parse_amount("12345678901234567890.1234");

        assert_eq!(amount, Ok(Decimal::from_str("12345678901234567890.1234").unwrap()));
    }

    #[test]
    fn parse_amount_accepts_comma_decimal_separator() {
        assert_eq!(parse_amount("12,50"), Ok(Decimal::new(1250, 2)));
    }

    #[test]
    fn parse_amount_rejects_mixed_separators() {
        assert_eq!(parse_amount("1,234.50"), Err(DomainError::InvalidAmountFormat));
    }

    #[test]
    fn parse_amount_rejects_ambiguous_thousands_separator() {
        assert_eq!(parse_amount("1,000"), Err(DomainError::InvalidAmountFormat));
    }

    #[test]
    fn parse_amount_rejects_zero_and_negative_values() {
        assert_eq!(parse_amount("0"), Err(DomainError::InvalidAmount));
        assert_eq!(parse_amount("-1"), Err(DomainError::InvalidAmountFormat));
    }

    #[test]
    fn parse_amount_expression_adds_multiple_amounts() {
        assert_eq!(parse_amount_expression("100 + 200 + 347"), Ok(Decimal::from(647)));
    }

    #[test]
    fn parse_amount_expression_adds_amounts_without_spaces() {
        assert_eq!(parse_amount_expression("100+23"), Ok(Decimal::from(123)));
    }

    #[test]
    fn parse_amount_expression_preserves_decimal_precision() {
        assert_eq!(parse_amount_expression("1,25 + 2.50"), Ok(Decimal::new(375, 2)));
    }

    #[test]
    fn parse_amount_expression_rejects_empty_terms_and_other_operators() {
        assert_eq!(parse_amount_expression("1 + + 2"), Err(DomainError::InvalidAmountFormat));
        assert_eq!(parse_amount_expression("1 - 2"), Err(DomainError::InvalidAmountFormat));
    }

    #[test]
    fn parse_amount_expression_reports_overflow() {
        assert_eq!(
            parse_amount_expression("79228162514264337593543950335 + 1"),
            Err(DomainError::AmountOverflow)
        );
    }

    #[test]
    fn canonical_amount_removes_unneeded_scale() {
        assert_eq!(canonical_amount(Decimal::new(1200, 2)), "12");
    }
}