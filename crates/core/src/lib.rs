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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavingsTerm {
    Limited,
    Unlimited,
}

impl SavingsTerm {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Limited => "limited",
            Self::Unlimited => "unlimited",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Limited => "С ограниченным сроком",
            Self::Unlimited => "Бессрочный",
        }
    }
}

impl FromStr for SavingsTerm {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "limited" => Ok(Self::Limited),
            "unlimited" => Ok(Self::Unlimited),
            _ => Err(DomainError::UnknownSavingsTerm(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterestDestination {
    Reinvest,
    Withdraw,
}

impl InterestDestination {
    pub const ALL: [Self; 2] = [Self::Reinvest, Self::Withdraw];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Reinvest => "reinvest",
            Self::Withdraw => "withdraw",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Reinvest => "Капитализация",
            Self::Withdraw => "На карту",
        }
    }
}

impl FromStr for InterestDestination {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "reinvest" => Ok(Self::Reinvest),
            "withdraw" => Ok(Self::Withdraw),
            _ => Err(DomainError::UnknownInterestDestination(value.to_owned())),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavingsAccount {
    pub id: i64,
    pub bank_id: i64,
    pub bank_name: String,
    pub title: String,
    pub amount: Decimal,
    pub base_rate: Decimal,
    pub rate_after_threshold: Option<Decimal>,
    pub threshold_amount: Option<Decimal>,
    pub term_type: SavingsTerm,
    pub term_end_date: Option<NaiveDate>,
    pub interest_destination: InterestDestination,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavingsDetails {
    pub bank_id: i64,
    pub title: String,
    pub amount: Decimal,
    pub base_rate: Decimal,
    pub rate_after_threshold: Option<Decimal>,
    pub threshold_amount: Option<Decimal>,
    pub term_type: SavingsTerm,
    pub term_end_date: Option<NaiveDate>,
    pub interest_destination: InterestDestination,
    pub is_active: bool,
}

impl SavingsDetails {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_savings(
            &self.title,
            self.amount,
            self.base_rate,
            self.rate_after_threshold,
            self.threshold_amount,
            self.term_type,
            self.term_end_date,
        )
    }
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
    #[error("Некорректное числовое значение")]
    InvalidDecimalFormat,
    #[error("Название накопления не может быть пустым")]
    InvalidSavingsTitle,
    #[error("Сумма накопления должна быть положительным числом")]
    InvalidSavingsAmount,
    #[error("Ставка не может быть отрицательной")]
    InvalidSavingsRate,
    #[error("Пороговая сумма должна быть положительным числом")]
    InvalidSavingsThresholdAmount,
    #[error("Укажите одновременно пороговую ставку и пороговую сумму")]
    IncompleteSavingsThreshold,
    #[error(
        "Дата окончания обязательна для вклада с ограниченным сроком и недопустима для бессрочного"
    )]
    InvalidSavingsTerm,
    #[error("Неизвестный тип срока накопления: {0}")]
    UnknownSavingsTerm(String),
    #[error("Неизвестное назначение процентов: {0}")]
    UnknownInterestDestination(String),
}

/// Parses a non-negative decimal without passing through binary floating point.
///
/// A comma is accepted as the decimal separator only when a dot is absent;
/// grouping separators and scientific notation are rejected.
pub fn parse_non_negative_decimal(input: &str) -> Result<Decimal, DomainError> {
    let value = input.trim();
    let ambiguous_grouping = value
        .split_once(',')
        .is_some_and(|(whole, fraction)| (1..=3).contains(&whole.len()) && fraction.len() == 3);
    if value.is_empty()
        || value.contains('.') && value.contains(',')
        || ambiguous_grouping
        || value
            .chars()
            .any(|character| !character.is_ascii_digit() && character != '.' && character != ',')
        || value.matches('.').count() + value.matches(',').count() > 1
    {
        return Err(DomainError::InvalidDecimalFormat);
    }

    Decimal::from_str(&value.replace(',', ".")).map_err(|_| DomainError::InvalidDecimalFormat)
}

/// Validates the domain constraints for a savings deposit or account.
pub fn validate_savings(
    title: &str,
    amount: Decimal,
    base_rate: Decimal,
    rate_after_threshold: Option<Decimal>,
    threshold_amount: Option<Decimal>,
    term_type: SavingsTerm,
    term_end_date: Option<NaiveDate>,
) -> Result<(), DomainError> {
    if title.trim().is_empty() {
        return Err(DomainError::InvalidSavingsTitle);
    }
    if amount <= Decimal::ZERO {
        return Err(DomainError::InvalidSavingsAmount);
    }
    if base_rate < Decimal::ZERO || rate_after_threshold.is_some_and(|rate| rate < Decimal::ZERO) {
        return Err(DomainError::InvalidSavingsRate);
    }
    match (rate_after_threshold, threshold_amount) {
        (Some(_), Some(threshold)) if threshold > Decimal::ZERO => {}
        (None, None) => {}
        (Some(_), Some(_)) => return Err(DomainError::InvalidSavingsThresholdAmount),
        _ => return Err(DomainError::IncompleteSavingsThreshold),
    }
    match (term_type, term_end_date) {
        (SavingsTerm::Limited, Some(_)) | (SavingsTerm::Unlimited, None) => Ok(()),
        _ => Err(DomainError::InvalidSavingsTerm),
    }
}

/// Parses a positive amount without passing through binary floating point.
///
/// A comma is accepted as the decimal separator only when a dot is absent;
/// grouping separators and scientific notation are rejected.
pub fn parse_amount(input: &str) -> Result<Decimal, DomainError> {
    let amount = parse_non_negative_decimal(input).map_err(|_| DomainError::InvalidAmountFormat)?;
    if amount <= Decimal::ZERO {
        return Err(DomainError::InvalidAmount);
    }
    Ok(amount)
}

/// Parses a sum of positive amounts separated by plus signs.
pub fn parse_amount_expression(input: &str) -> Result<Decimal, DomainError> {
    input.split('+').try_fold(Decimal::ZERO, |total, term| {
        let amount = parse_amount(term)?;
        total.checked_add(amount).ok_or(DomainError::AmountOverflow)
    })
}

/// Formats a decimal amount in a locale-independent representation for SQLite.
pub fn canonical_amount(amount: Decimal) -> String {
    amount.normalize().to_string()
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::{
        canonical_amount, parse_amount, parse_amount_expression, parse_non_negative_decimal,
        validate_savings, DomainError, InterestDestination, SavingsTerm,
    };

    #[test]
    fn parse_amount_preserves_decimal_precision() {
        let amount = parse_amount("12345678901234567890.1234");

        assert_eq!(
            amount,
            Ok(Decimal::from_str("12345678901234567890.1234").unwrap())
        );
    }

    #[test]
    fn parse_amount_accepts_comma_decimal_separator() {
        assert_eq!(parse_amount("12,50"), Ok(Decimal::new(1250, 2)));
    }

    #[test]
    fn parse_amount_rejects_mixed_separators() {
        assert_eq!(
            parse_amount("1,234.50"),
            Err(DomainError::InvalidAmountFormat)
        );
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
        assert_eq!(
            parse_amount_expression("100 + 200 + 347"),
            Ok(Decimal::from(647))
        );
    }

    #[test]
    fn parse_amount_expression_adds_amounts_without_spaces() {
        assert_eq!(parse_amount_expression("100+23"), Ok(Decimal::from(123)));
    }

    #[test]
    fn parse_amount_expression_preserves_decimal_precision() {
        assert_eq!(
            parse_amount_expression("1,25 + 2.50"),
            Ok(Decimal::new(375, 2))
        );
    }

    #[test]
    fn parse_amount_expression_rejects_empty_terms_and_other_operators() {
        assert_eq!(
            parse_amount_expression("1 + + 2"),
            Err(DomainError::InvalidAmountFormat)
        );
        assert_eq!(
            parse_amount_expression("1 - 2"),
            Err(DomainError::InvalidAmountFormat)
        );
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

    #[test]
    fn parse_non_negative_decimal_accepts_zero_and_comma_separator() {
        assert_eq!(parse_non_negative_decimal("0"), Ok(Decimal::ZERO));
        assert_eq!(
            parse_non_negative_decimal("12,50"),
            Ok(Decimal::new(1250, 2))
        );
    }

    #[test]
    fn parse_non_negative_decimal_rejects_ambiguous_or_signed_values() {
        assert_eq!(
            parse_non_negative_decimal("1,000"),
            Err(DomainError::InvalidDecimalFormat)
        );
        assert_eq!(
            parse_non_negative_decimal("-1"),
            Err(DomainError::InvalidDecimalFormat)
        );
    }

    #[test]
    fn validate_savings_rejects_unpaired_threshold_fields() {
        assert_eq!(
            validate_savings(
                "Вклад",
                Decimal::ONE,
                Decimal::ZERO,
                Some(Decimal::ONE),
                None,
                SavingsTerm::Unlimited,
                None,
            ),
            Err(DomainError::IncompleteSavingsThreshold)
        );
    }

    #[test]
    fn validate_savings_rejects_invalid_term_date_combinations() {
        assert_eq!(
            validate_savings(
                "Вклад",
                Decimal::ONE,
                Decimal::ZERO,
                None,
                None,
                SavingsTerm::Limited,
                None,
            ),
            Err(DomainError::InvalidSavingsTerm)
        );
    }

    #[test]
    fn validate_savings_accepts_valid_limited_term_and_threshold() {
        assert_eq!(
            validate_savings(
                "Вклад",
                Decimal::new(100_000, 2),
                Decimal::new(5, 0),
                Some(Decimal::new(7, 0)),
                Some(Decimal::new(50_000, 2)),
                SavingsTerm::Limited,
                NaiveDate::from_ymd_opt(2027, 1, 1),
            ),
            Ok(())
        );
    }

    #[test]
    fn savings_enum_storage_codes_are_explicit() {
        assert_eq!(SavingsTerm::Limited.code(), "limited");
        assert_eq!(
            "unlimited".parse::<SavingsTerm>(),
            Ok(SavingsTerm::Unlimited)
        );
        assert_eq!(InterestDestination::Reinvest.code(), "reinvest");
        assert_eq!(
            "withdraw".parse::<InterestDestination>(),
            Ok(InterestDestination::Withdraw)
        );
    }
}
