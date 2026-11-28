use std::str::FromStr;

use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// Фильтр операции по способу получения/списания через платёжную карту.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PaymentDocumentFilter {
    /// Все операции, независимо от наличия карты.
    #[default]
    All,
    /// Только операции без привязанной карты.
    WithoutCard,
    /// Только операции с указанной картой.
    Card(i64),
}

impl PaymentDocumentFilter {
    pub const fn card_id(self) -> Option<i64> {
        match self {
            Self::Card(id) => Some(id),
            _ => None,
        }
    }

    pub const fn is_active(self) -> bool {
        !matches!(self, Self::All)
    }
}

/// Фильтр расходов по признаку регулярности категории.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RegularExpenseFilter {
    /// Все операции, независимо от признака категории.
    #[default]
    All,
    /// Только операции по регулярным категориям расходов.
    Regular,
    /// Только операции по нерегулярным категориям расходов.
    NotRegular,
}

impl RegularExpenseFilter {
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::All)
    }

    pub const fn matches(self, is_regular: bool) -> bool {
        match self {
            Self::All => true,
            Self::Regular => is_regular,
            Self::NotRegular => !is_regular,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// A currency's stored exchange rate relative to the application's primary currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyDefinition {
    pub code: String,
    pub name: String,
    pub is_primary: bool,
    pub rate_to_primary: Option<Decimal>,
    pub rate_date: Option<NaiveDate>,
}

/// Maximum scale accepted for a manually entered exchange rate.
pub const MAX_EXCHANGE_RATE_SCALE: u32 = 12;
const REBASED_EXCHANGE_RATE_SCALE: u32 = 24;

/// Validates an ISO-style three-letter uppercase currency code.
pub fn validate_currency_code(code: &str) -> Result<(), DomainError> {
    if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(DomainError::InvalidCurrencyCode);
    }
    Ok(())
}

/// Validates a positive exchange rate with the supported decimal scale.
pub fn validate_exchange_rate(rate: Decimal) -> Result<(), DomainError> {
    if rate <= Decimal::ZERO {
        return Err(DomainError::InvalidExchangeRate);
    }
    if rate.scale() > MAX_EXCHANGE_RATE_SCALE {
        return Err(DomainError::ExchangeRatePrecision);
    }
    Ok(())
}

/// Converts a currency's rate to a new primary currency's units.
///
/// Both rates use the same old primary currency as their denominator. The returned rate
/// is rounded to 24 places using midpoint-to-even rounding.
pub fn rebase_exchange_rate(
    rate: Decimal,
    new_primary_rate: Decimal,
) -> Result<Decimal, DomainError> {
    if rate <= Decimal::ZERO || new_primary_rate <= Decimal::ZERO {
        return Err(DomainError::InvalidExchangeRate);
    }
    let rebased = rate
        .checked_div(new_primary_rate)
        .ok_or(DomainError::ExchangeRateOverflow)?
        .round_dp_with_strategy(
            REBASED_EXCHANGE_RATE_SCALE,
            RoundingStrategy::MidpointNearestEven,
        );
    if rebased <= Decimal::ZERO {
        return Err(DomainError::ExchangeRatePrecision);
    }
    Ok(rebased)
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
    Other,
}

impl InterestDestination {
    pub const ALL: [Self; 3] = [Self::Reinvest, Self::Withdraw, Self::Other];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Reinvest => "reinvest",
            Self::Withdraw => "withdraw",
            Self::Other => "other",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Reinvest => "Капитализация",
            Self::Withdraw => "На карту",
            Self::Other => "Другое",
        }
    }
}

impl FromStr for InterestDestination {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "reinvest" => Ok(Self::Reinvest),
            "withdraw" => Ok(Self::Withdraw),
            "other" => Ok(Self::Other),
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
    /// `Some(...)` только для категорий расходов; у доходов флага нет в БД.
    pub is_regular: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bank {
    pub id: i64,
    pub name: String,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpensePlace {
    pub id: i64,
    pub name: String,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceRecord {
    pub id: i64,
    pub name: String,
    pub date: NaiveDate,
    pub amount: Decimal,
    pub currency: Currency,
    pub expense_place_id: i64,
    pub expense_place_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PriceFilter {
    pub currency: Option<Currency>,
    pub expense_place_id: Option<i64>,
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    /// Необязательный поиск по названию товара или месту покупки.
    pub search: String,
}

impl PriceFilter {
    pub fn validate(&self) -> Result<(), DomainError> {
        if matches!((self.date_from, self.date_to), (Some(from), Some(to)) if from > to) {
            return Err(DomainError::InvalidPriceDateRange);
        }
        Ok(())
    }
}

pub fn validate_price(name: &str, amount: Decimal) -> Result<(), DomainError> {
    if name.trim().is_empty() {
        return Err(DomainError::InvalidPriceName);
    }
    if amount <= Decimal::ZERO {
        return Err(DomainError::InvalidAmount);
    }
    if amount.scale() > 12 {
        return Err(DomainError::InvalidPricePrecision);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialTransaction {
    pub id: i64,
    pub date: NaiveDate,
    pub category_id: i64,
    pub category_name: String,
    /// Текущий признак регулярности категории; для доходов всегда `false`.
    pub is_regular: bool,
    pub amount: Decimal,
    pub currency: Currency,
    pub comment: String,
    pub expense_place_id: Option<i64>,
    pub expense_place_name: Option<String>,
    pub payment_document_id: Option<i64>,
    pub affects_deposit_rate: bool,
    pub payment_document_balance_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisTransaction {
    pub kind: TransactionKind,
    pub category_id: i64,
    pub category_name: String,
    pub amount: Decimal,
    pub currency: Currency,
    pub is_regular: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisRegularTotal {
    pub amount: Decimal,
    pub transaction_count: usize,
    pub share_percent: Decimal,
}

impl AnalysisRegularTotal {
    fn zero() -> Self {
        Self {
            amount: Decimal::ZERO,
            transaction_count: 0,
            share_percent: Decimal::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisCategoryTotal {
    pub kind: TransactionKind,
    pub category_id: i64,
    pub category_name: String,
    pub currency: Currency,
    pub amount: Decimal,
    pub transaction_count: usize,
    pub share_percent: Decimal,
    /// Признак регулярности категории расходов; для доходов всегда `false`.
    pub is_regular: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisTotal {
    pub kind: TransactionKind,
    pub currency: Currency,
    pub amount: Decimal,
    pub transaction_count: usize,
    /// `Some` для расходов, включая нулевой итог; `None` для доходов.
    pub regular: Option<AnalysisRegularTotal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisSummary {
    pub categories: Vec<AnalysisCategoryTotal>,
    pub totals: Vec<AnalysisTotal>,
}

/// Разрезы detailed-аналитики. Общая аналитика по категориям остаётся в
/// [`summarize_analysis`] и не заменяется этими отчётами.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisSlice {
    Overall,
    Cards,
    Banks,
    Deposits,
    Places,
    Currency,
}

impl AnalysisSlice {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Overall => "Общая аналитика",
            Self::Cards => "По карточкам",
            Self::Banks => "По банкам",
            Self::Deposits => "По вкладам",
            Self::Places => "По местам трат",
            Self::Currency => "По валюте",
        }
    }
}

/// Стабильный идентификатор группы для операций без связанной записи.
pub const ANALYSIS_UNSPECIFIED_GROUP_ID: i64 = 0;

/// Одна строка исходных данных detailed-аналитики.
///
/// `group_id` — устойчивый идентификатор группы, а не её название: разные
/// записи с одинаковыми названиями не объединяются.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisSliceItem {
    pub group_id: i64,
    pub group_name: String,
    pub currency: Currency,
    pub amount: Decimal,
}

impl AnalysisSliceItem {
    /// Группа валюты: каждая валюта образует отдельную группу со своей шкалой.
    pub fn currency_group(currency: Currency, amount: Decimal) -> Self {
        Self {
            group_id: match currency {
                Currency::Rub => 1,
                Currency::Usd => 2,
            },
            group_name: currency.code().to_owned(),
            currency,
            amount,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisSliceRow {
    pub group_id: i64,
    pub group_name: String,
    pub currency: Currency,
    pub amount: Decimal,
    pub transaction_count: usize,
    /// Доля от итога той же валюты в этом отчёте.
    pub share_percent: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisSliceTotal {
    pub currency: Currency,
    pub amount: Decimal,
    pub transaction_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisSliceReport {
    pub rows: Vec<AnalysisSliceRow>,
    pub totals: Vec<AnalysisSliceTotal>,
}

/// Typed query shared by the analysis screen and the read-only database API.
#[derive(Debug, Clone)]
pub struct AnalysisSliceQuery {
    pub kind: TransactionKind,
    pub slice: AnalysisSlice,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub category_ids: Option<Vec<i64>>,
    pub currency: Option<Currency>,
    pub search: String,
    pub bank_id: Option<i64>,
    pub payment_document_filter: PaymentDocumentFilter,
    pub expense_place_id: Option<i64>,
    pub regular_filter: RegularExpenseFilter,
}

/// Aggregates already filtered report items by stable group and currency.
///
/// Sums and shares are exact `Decimal` values. Shares are computed only inside
/// one currency; RUB and USD are never added together.
pub fn summarize_analysis_slice(
    items: &[AnalysisSliceItem],
) -> Result<AnalysisSliceReport, AnalysisError> {
    use std::collections::BTreeMap;

    let mut groups = BTreeMap::<(Currency, i64), AnalysisSliceRow>::new();
    let mut totals = BTreeMap::<Currency, AnalysisSliceTotal>::new();

    for item in items {
        if item.amount <= Decimal::ZERO {
            return Err(AnalysisError::InvalidAmount);
        }
        let key = (item.currency, item.group_id);
        let group = groups.entry(key).or_insert_with(|| AnalysisSliceRow {
            group_id: item.group_id,
            group_name: item.group_name.clone(),
            currency: item.currency,
            amount: Decimal::ZERO,
            transaction_count: 0,
            share_percent: Decimal::ZERO,
        });
        group.amount = group
            .amount
            .checked_add(item.amount)
            .ok_or(AnalysisError::Overflow)?;
        group.transaction_count = group
            .transaction_count
            .checked_add(1)
            .ok_or(AnalysisError::Overflow)?;

        let total = totals
            .entry(item.currency)
            .or_insert_with(|| AnalysisSliceTotal {
                currency: item.currency,
                amount: Decimal::ZERO,
                transaction_count: 0,
            });
        total.amount = total
            .amount
            .checked_add(item.amount)
            .ok_or(AnalysisError::Overflow)?;
        total.transaction_count = total
            .transaction_count
            .checked_add(1)
            .ok_or(AnalysisError::Overflow)?;
    }

    for row in groups.values_mut() {
        let total = totals
            .get(&row.currency)
            .ok_or(AnalysisError::MissingTotal)?;
        row.share_percent = row
            .amount
            .checked_div(total.amount)
            .and_then(|share| share.checked_mul(Decimal::ONE_HUNDRED))
            .ok_or(AnalysisError::Overflow)?;
    }

    Ok(AnalysisSliceReport {
        rows: groups.into_values().collect(),
        totals: totals.into_values().collect(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionAmount {
    pub kind: TransactionKind,
    pub amount: Decimal,
    pub currency: Currency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionBalance {
    pub currency: Currency,
    pub income: Decimal,
    pub expenses: Decimal,
    pub balance: Decimal,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AnalysisError {
    #[error("Сумма операции для анализа должна быть положительной")]
    InvalidAmount,
    #[error("Переполнение при расчёте анализа")]
    Overflow,
    #[error("Не удалось определить итог для доли категории")]
    MissingTotal,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransactionBalanceError {
    #[error("Сумма операции для расчёта сальдо должна быть положительной")]
    InvalidAmount,
    #[error("Переполнение при расчёте сальдо")]
    Overflow,
}

/// Aggregates transactions by kind, category, and currency without conversion.
pub fn summarize_analysis(
    transactions: &[AnalysisTransaction],
) -> Result<AnalysisSummary, AnalysisError> {
    use std::collections::BTreeMap;

    let mut categories = BTreeMap::<(TransactionKind, Currency, i64), AnalysisCategoryTotal>::new();
    let mut totals = BTreeMap::<(TransactionKind, Currency), AnalysisTotal>::new();

    for transaction in transactions {
        if transaction.amount <= Decimal::ZERO {
            return Err(AnalysisError::InvalidAmount);
        }

        let category = categories
            .entry((
                transaction.kind,
                transaction.currency,
                transaction.category_id,
            ))
            .or_insert_with(|| AnalysisCategoryTotal {
                kind: transaction.kind,
                category_id: transaction.category_id,
                category_name: transaction.category_name.clone(),
                currency: transaction.currency,
                amount: Decimal::ZERO,
                transaction_count: 0,
                share_percent: Decimal::ZERO,
                is_regular: transaction.kind == TransactionKind::Expense && transaction.is_regular,
            });
        category.amount = category
            .amount
            .checked_add(transaction.amount)
            .ok_or(AnalysisError::Overflow)?;
        category.transaction_count = category
            .transaction_count
            .checked_add(1)
            .ok_or(AnalysisError::Overflow)?;

        let total = totals
            .entry((transaction.kind, transaction.currency))
            .or_insert_with(|| AnalysisTotal {
                kind: transaction.kind,
                currency: transaction.currency,
                amount: Decimal::ZERO,
                transaction_count: 0,
                regular: (transaction.kind == TransactionKind::Expense)
                    .then(AnalysisRegularTotal::zero),
            });
        total.amount = total
            .amount
            .checked_add(transaction.amount)
            .ok_or(AnalysisError::Overflow)?;
        total.transaction_count = total
            .transaction_count
            .checked_add(1)
            .ok_or(AnalysisError::Overflow)?;
        if transaction.kind == TransactionKind::Expense && transaction.is_regular {
            let regular = total.regular.get_or_insert_with(AnalysisRegularTotal::zero);
            regular.amount = regular
                .amount
                .checked_add(transaction.amount)
                .ok_or(AnalysisError::Overflow)?;
            regular.transaction_count = regular
                .transaction_count
                .checked_add(1)
                .ok_or(AnalysisError::Overflow)?;
        }
    }

    for category in categories.values_mut() {
        let total = totals
            .get(&(category.kind, category.currency))
            .ok_or(AnalysisError::MissingTotal)?;
        category.share_percent = category
            .amount
            .checked_div(total.amount)
            .and_then(|share| share.checked_mul(Decimal::ONE_HUNDRED))
            .ok_or(AnalysisError::Overflow)?;
    }

    for total in totals.values_mut() {
        let Some(regular) = total.regular.as_mut() else {
            continue;
        };
        let total_amount = total.amount;
        regular.share_percent = regular
            .amount
            .checked_div(total_amount)
            .and_then(|share| share.checked_mul(Decimal::ONE_HUNDRED))
            .ok_or(AnalysisError::Overflow)?;
    }

    Ok(AnalysisSummary {
        categories: categories.into_values().collect(),
        totals: totals.into_values().collect(),
    })
}

/// Aggregates income, expenses, and balance by currency without conversion.
pub fn summarize_transaction_balance(
    transactions: &[TransactionAmount],
) -> Result<[TransactionBalance; 2], TransactionBalanceError> {
    let mut totals = Currency::ALL.map(|currency| TransactionBalance {
        currency,
        income: Decimal::ZERO,
        expenses: Decimal::ZERO,
        balance: Decimal::ZERO,
    });

    for transaction in transactions {
        if transaction.amount <= Decimal::ZERO {
            return Err(TransactionBalanceError::InvalidAmount);
        }

        let total = &mut totals[match transaction.currency {
            Currency::Rub => 0,
            Currency::Usd => 1,
        }];
        let amount = match transaction.kind {
            TransactionKind::Income => &mut total.income,
            TransactionKind::Expense => &mut total.expenses,
        };
        *amount = amount
            .checked_add(transaction.amount)
            .ok_or(TransactionBalanceError::Overflow)?;
    }

    for total in &mut totals {
        total.balance = total
            .income
            .checked_sub(total.expenses)
            .ok_or(TransactionBalanceError::Overflow)?;
    }

    Ok(totals)
}

#[cfg(test)]
mod analysis_tests {
    use rust_decimal::Decimal;

    use super::{
        summarize_analysis, AnalysisError, AnalysisTransaction, Currency, TransactionKind,
    };

    #[test]
    fn summarize_analysis_separates_categories_by_transaction_kind() {
        let transactions = [
            AnalysisTransaction {
                kind: TransactionKind::Income,
                category_id: 1,
                category_name: "Категория дохода".to_owned(),
                amount: Decimal::from(30),
                currency: Currency::Rub,
                is_regular: false,
            },
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 1,
                category_name: "Категория расхода".to_owned(),
                amount: Decimal::from(20),
                currency: Currency::Rub,
                is_regular: false,
            },
        ];

        let summary = summarize_analysis(&transactions).unwrap();

        assert_eq!(summary.categories.len(), 2);
        assert_eq!(summary.totals.len(), 2);
        assert_ne!(summary.categories[0].kind, summary.categories[1].kind);
    }

    #[test]
    fn summarize_analysis_keeps_currency_totals_separate() {
        let transactions = [
            AnalysisTransaction {
                kind: TransactionKind::Income,
                category_id: 1,
                category_name: "Зарплата".to_owned(),
                amount: Decimal::new(1250, 2),
                currency: Currency::Rub,
                is_regular: false,
            },
            AnalysisTransaction {
                kind: TransactionKind::Income,
                category_id: 1,
                category_name: "Зарплата".to_owned(),
                amount: Decimal::from(5),
                currency: Currency::Usd,
                is_regular: false,
            },
        ];

        let summary = summarize_analysis(&transactions).unwrap();

        assert_eq!(summary.totals.len(), 2);
        assert_eq!(summary.categories.len(), 2);
        assert_eq!(summary.categories[0].share_percent, Decimal::from(100));
    }

    #[test]
    fn summarize_analysis_aggregates_exact_amounts_and_shares() {
        let transactions = [
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 1,
                category_name: "Жильё".to_owned(),
                amount: Decimal::new(1250, 2),
                currency: Currency::Rub,
                is_regular: false,
            },
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 1,
                category_name: "Жильё".to_owned(),
                amount: Decimal::new(750, 2),
                currency: Currency::Rub,
                is_regular: false,
            },
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 2,
                category_name: "Питание".to_owned(),
                amount: Decimal::from(5),
                currency: Currency::Rub,
                is_regular: false,
            },
        ];

        let summary = summarize_analysis(&transactions).unwrap();

        assert_eq!(summary.categories[0].amount, Decimal::from(20));
        assert_eq!(summary.categories[0].transaction_count, 2);
        assert_eq!(summary.categories[0].share_percent, Decimal::from(80));
        assert_eq!(summary.totals[0].amount, Decimal::from(25));
        assert_eq!(summary.totals[0].transaction_count, 3);
    }

    #[test]
    fn summarize_analysis_returns_empty_summary_for_no_transactions() {
        assert_eq!(
            summarize_analysis(&[]),
            Ok(super::AnalysisSummary {
                categories: Vec::new(),
                totals: Vec::new(),
            })
        );
    }

    #[test]
    fn summarize_analysis_reports_amount_overflow() {
        let transactions = [
            AnalysisTransaction {
                kind: TransactionKind::Income,
                category_id: 1,
                category_name: "Зарплата".to_owned(),
                amount: Decimal::MAX,
                currency: Currency::Rub,
                is_regular: false,
            },
            AnalysisTransaction {
                kind: TransactionKind::Income,
                category_id: 1,
                category_name: "Зарплата".to_owned(),
                amount: Decimal::ONE,
                currency: Currency::Rub,
                is_regular: false,
            },
        ];

        assert_eq!(
            summarize_analysis(&transactions),
            Err(AnalysisError::Overflow)
        );
    }

    #[test]
    fn summarize_analysis_rejects_non_positive_amounts() {
        let transaction = AnalysisTransaction {
            kind: TransactionKind::Income,
            category_id: 1,
            category_name: "Зарплата".to_owned(),
            amount: Decimal::ZERO,
            currency: Currency::Rub,
            is_regular: false,
        };

        assert_eq!(
            summarize_analysis(&[transaction]),
            Err(AnalysisError::InvalidAmount)
        );
    }

    #[test]
    fn regular_total_counts_only_marked_expenses() {
        let transactions = [
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 1,
                category_name: "Жильё".to_owned(),
                amount: Decimal::from(30),
                currency: Currency::Rub,
                is_regular: true,
            },
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 2,
                category_name: "Питание".to_owned(),
                amount: Decimal::from(10),
                currency: Currency::Rub,
                is_regular: false,
            },
            AnalysisTransaction {
                kind: TransactionKind::Income,
                category_id: 3,
                category_name: "Зарплата".to_owned(),
                amount: Decimal::from(100),
                currency: Currency::Rub,
                is_regular: true,
            },
        ];

        let summary = summarize_analysis(&transactions).unwrap();
        let expense = summary
            .totals
            .iter()
            .find(|total| total.kind == TransactionKind::Expense)
            .unwrap();
        let regular = expense.regular.as_ref().unwrap();
        assert_eq!(regular.amount, Decimal::from(30));
        assert_eq!(regular.transaction_count, 1);
        assert_eq!(regular.share_percent, Decimal::from(75));
        assert!(summary
            .totals
            .iter()
            .find(|total| total.kind == TransactionKind::Income)
            .unwrap()
            .regular
            .is_none());
    }

    #[test]
    fn category_totals_keep_the_expense_regular_flag() {
        let transactions = [
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 1,
                category_name: "Жильё".to_owned(),
                amount: Decimal::from(30),
                currency: Currency::Rub,
                is_regular: true,
            },
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 2,
                category_name: "Питание".to_owned(),
                amount: Decimal::from(10),
                currency: Currency::Rub,
                is_regular: false,
            },
            AnalysisTransaction {
                kind: TransactionKind::Income,
                category_id: 3,
                category_name: "Зарплата".to_owned(),
                amount: Decimal::from(100),
                currency: Currency::Rub,
                is_regular: true,
            },
        ];

        let summary = summarize_analysis(&transactions).unwrap();

        let find = |id| {
            summary
                .categories
                .iter()
                .find(|category| category.category_id == id)
                .unwrap()
                .is_regular
        };
        assert!(find(1));
        assert!(!find(2));
        assert!(!find(3));
    }

    #[test]
    fn regular_total_keeps_currencies_separate() {
        let transactions = [
            (TransactionKind::Expense, Currency::Rub, 8, true),
            (TransactionKind::Expense, Currency::Rub, 2, false),
            (TransactionKind::Expense, Currency::Usd, 1, true),
            (TransactionKind::Expense, Currency::Usd, 3, false),
        ]
        .into_iter()
        .enumerate()
        .map(
            |(index, (kind, currency, amount, is_regular))| AnalysisTransaction {
                kind,
                category_id: index as i64,
                category_name: format!("Категория {index}"),
                amount: Decimal::from(amount),
                currency,
                is_regular,
            },
        )
        .collect::<Vec<_>>();

        let summary = summarize_analysis(&transactions).unwrap();
        let rub = summary
            .totals
            .iter()
            .find(|total| total.currency == Currency::Rub)
            .unwrap();
        let usd = summary
            .totals
            .iter()
            .find(|total| total.currency == Currency::Usd)
            .unwrap();
        assert_eq!(rub.regular.as_ref().unwrap().amount, Decimal::from(8));
        assert_eq!(
            rub.regular.as_ref().unwrap().share_percent,
            Decimal::from(80)
        );
        assert_eq!(usd.regular.as_ref().unwrap().amount, Decimal::ONE);
        assert_eq!(
            usd.regular.as_ref().unwrap().share_percent,
            Decimal::from(25)
        );
    }

    #[test]
    fn regular_total_is_zero_when_no_expense_is_marked() {
        let transactions = [AnalysisTransaction {
            kind: TransactionKind::Expense,
            category_id: 1,
            category_name: "Питание".to_owned(),
            amount: Decimal::from(5),
            currency: Currency::Rub,
            is_regular: false,
        }];

        let summary = summarize_analysis(&transactions).unwrap();
        let regular = summary.totals[0].regular.as_ref().unwrap();
        assert_eq!(regular.amount, Decimal::ZERO);
        assert_eq!(regular.transaction_count, 0);
        assert_eq!(regular.share_percent, Decimal::ZERO);
    }

    #[test]
    fn regular_total_reports_overflow() {
        let transactions = [
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 1,
                category_name: "Жильё".to_owned(),
                amount: Decimal::MAX,
                currency: Currency::Rub,
                is_regular: true,
            },
            AnalysisTransaction {
                kind: TransactionKind::Expense,
                category_id: 1,
                category_name: "Жильё".to_owned(),
                amount: Decimal::ONE,
                currency: Currency::Rub,
                is_regular: true,
            },
        ];

        assert_eq!(
            summarize_analysis(&transactions),
            Err(AnalysisError::Overflow)
        );
    }
}

#[cfg(test)]
mod analysis_slice_tests {
    use std::str::FromStr;

    use super::{
        summarize_analysis_slice, AnalysisError, AnalysisSliceItem, Currency,
        ANALYSIS_UNSPECIFIED_GROUP_ID,
    };
    use rust_decimal::Decimal;

    fn item(id: i64, name: &str, currency: Currency, amount: &str) -> AnalysisSliceItem {
        AnalysisSliceItem {
            group_id: id,
            group_name: name.to_owned(),
            currency,
            amount: amount.parse().unwrap(),
        }
    }

    #[test]
    fn summarize_analysis_slice_keeps_identical_names_with_different_ids_separate() {
        let report = summarize_analysis_slice(&[
            item(1, "Одинаково", Currency::Rub, "10.25"),
            item(2, "Одинаково", Currency::Rub, "5.75"),
            item(
                ANALYSIS_UNSPECIFIED_GROUP_ID,
                "Не указано",
                Currency::Rub,
                "4",
            ),
        ])
        .unwrap();

        assert_eq!(report.rows.len(), 3);
        let total = &report.totals[0];
        assert_eq!(total.amount, Decimal::from(20));
        assert_eq!(total.transaction_count, 3);
        let rub_rows: Vec<_> = report
            .rows
            .iter()
            .map(|row| {
                (
                    row.group_id,
                    row.amount,
                    row.transaction_count,
                    row.share_percent,
                )
            })
            .collect();
        assert_eq!(
            rub_rows,
            vec![
                (0, Decimal::from(4), 1, Decimal::from(20)),
                (
                    1,
                    Decimal::from_str("10.25").unwrap(),
                    1,
                    Decimal::from_str("51.25").unwrap()
                ),
                (
                    2,
                    Decimal::from_str("5.75").unwrap(),
                    1,
                    Decimal::from_str("28.75").unwrap()
                ),
            ]
        );
    }

    #[test]
    fn summarize_analysis_slice_computes_shares_inside_each_currency_only() {
        let report = summarize_analysis_slice(&[
            AnalysisSliceItem::currency_group(Currency::Rub, Decimal::from(3)),
            AnalysisSliceItem::currency_group(Currency::Rub, Decimal::from(1)),
            AnalysisSliceItem::currency_group(Currency::Usd, Decimal::from(7)),
        ])
        .unwrap();

        assert_eq!(report.totals.len(), 2);
        let rub = report
            .rows
            .iter()
            .find(|row| row.currency == Currency::Rub)
            .unwrap();
        let usd = report
            .rows
            .iter()
            .find(|row| row.currency == Currency::Usd)
            .unwrap();
        assert_eq!(rub.share_percent, Decimal::ONE_HUNDRED);
        assert_eq!(usd.share_percent, Decimal::ONE_HUNDRED);
        assert_eq!(rub.amount, Decimal::from(4));
        assert_eq!(usd.amount, Decimal::from(7));
    }

    #[test]
    fn summarize_analysis_slice_returns_empty_report_for_empty_items() {
        assert_eq!(
            summarize_analysis_slice(&[]),
            Ok(super::AnalysisSliceReport {
                rows: Vec::new(),
                totals: Vec::new(),
            })
        );
    }

    #[test]
    fn summarize_analysis_slice_rejects_invalid_and_overflowing_amounts() {
        assert_eq!(
            summarize_analysis_slice(&[item(1, "Отрицательная", Currency::Rub, "-1")]),
            Err(AnalysisError::InvalidAmount)
        );
        assert_eq!(
            summarize_analysis_slice(&[
                item(1, "Максимум", Currency::Rub, &Decimal::MAX.to_string()),
                item(2, "Ещё", Currency::Rub, "1"),
            ]),
            Err(AnalysisError::Overflow)
        );
    }
}

#[cfg(test)]
mod transaction_balance_tests {
    use rust_decimal::Decimal;

    use super::{
        summarize_transaction_balance, Currency, TransactionAmount, TransactionBalanceError,
        TransactionKind,
    };

    #[test]
    fn summarize_transaction_balance_keeps_currencies_separate_and_allows_negative_balance() {
        let transactions = [
            TransactionAmount {
                kind: TransactionKind::Income,
                amount: Decimal::new(125_075, 2),
                currency: Currency::Rub,
            },
            TransactionAmount {
                kind: TransactionKind::Expense,
                amount: Decimal::from(1_500),
                currency: Currency::Rub,
            },
            TransactionAmount {
                kind: TransactionKind::Income,
                amount: Decimal::from(10),
                currency: Currency::Usd,
            },
            TransactionAmount {
                kind: TransactionKind::Expense,
                amount: Decimal::from(3),
                currency: Currency::Usd,
            },
        ];

        let summary = summarize_transaction_balance(&transactions).unwrap();

        assert_eq!(summary[0].balance, Decimal::new(-24_925, 2));
        assert_eq!(summary[1].balance, Decimal::from(7));
    }

    #[test]
    fn summarize_transaction_balance_returns_zero_for_empty_currencies() {
        let summary = summarize_transaction_balance(&[]).unwrap();

        assert_eq!(summary[0].income, Decimal::ZERO);
        assert_eq!(summary[0].expenses, Decimal::ZERO);
        assert_eq!(summary[0].balance, Decimal::ZERO);
        assert_eq!(summary[1].income, Decimal::ZERO);
        assert_eq!(summary[1].expenses, Decimal::ZERO);
        assert_eq!(summary[1].balance, Decimal::ZERO);
    }

    #[test]
    fn summarize_transaction_balance_reports_amount_overflow() {
        let transactions = [
            TransactionAmount {
                kind: TransactionKind::Income,
                amount: Decimal::MAX,
                currency: Currency::Rub,
            },
            TransactionAmount {
                kind: TransactionKind::Income,
                amount: Decimal::ONE,
                currency: Currency::Rub,
            },
        ];

        let error = summarize_transaction_balance(&transactions).unwrap_err();

        assert_eq!(error, TransactionBalanceError::Overflow);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavingsAccount {
    pub id: i64,
    pub bank_id: i64,
    pub bank_name: String,
    pub title: String,
    pub amount: Decimal,
    pub currency: Option<Currency>,
    pub base_rate: Decimal,
    pub rate_after_threshold: Option<Decimal>,
    pub threshold_amount: Option<Decimal>,
    pub opened_date: Option<NaiveDate>,
    pub interest_accrual_date: Option<NaiveDate>,
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
    pub currency: Option<Currency>,
    pub base_rate: Decimal,
    pub rate_after_threshold: Option<Decimal>,
    pub threshold_amount: Option<Decimal>,
    pub opened_date: Option<NaiveDate>,
    pub interest_accrual_date: Option<NaiveDate>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentSystem {
    Visa,
    MasterCard,
    Mir,
}

impl PaymentSystem {
    pub const ALL: [Self; 3] = [Self::Visa, Self::MasterCard, Self::Mir];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Visa => "VISA",
            Self::MasterCard => "MasterCard",
            Self::Mir => "MIR",
        }
    }
}

impl FromStr for PaymentSystem {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "VISA" => Ok(Self::Visa),
            "MasterCard" => Ok(Self::MasterCard),
            "MIR" => Ok(Self::Mir),
            _ => Err(DomainError::UnknownPaymentSystem(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentCardType {
    Debit,
    Credit,
}

impl PaymentCardType {
    pub const ALL: [Self; 2] = [Self::Debit, Self::Credit];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Debit => "debit",
            Self::Credit => "credit",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Debit => "Дебетовая",
            Self::Credit => "Кредитная",
        }
    }
}

impl FromStr for PaymentCardType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "debit" => Ok(Self::Debit),
            "credit" => Ok(Self::Credit),
            _ => Err(DomainError::UnknownPaymentCardType(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSavingsAccount {
    pub id: i64,
    pub title: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentDocument {
    pub id: i64,
    pub name: Option<String>,
    pub bank_id: i64,
    pub bank_name: String,
    pub payment_system: PaymentSystem,
    pub card_type: PaymentCardType,
    pub amount: Decimal,
    pub currency: Option<Currency>,
    pub due_date: Option<NaiveDate>,
    pub grace_period_days: Option<i64>,
    pub next_payment_amount: Option<Decimal>,
    pub affects_deposit_rate: bool,
    pub min_monthly_spend: Option<Decimal>,
    pub linked_savings: Vec<LinkedSavingsAccount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentDocumentDetails {
    pub name: Option<String>,
    pub bank_id: i64,
    pub payment_system: PaymentSystem,
    pub card_type: PaymentCardType,
    pub amount: Decimal,
    pub currency: Option<Currency>,
    pub due_date: Option<NaiveDate>,
    pub grace_period_days: Option<i64>,
    pub next_payment_amount: Option<Decimal>,
    pub affects_deposit_rate: bool,
    pub min_monthly_spend: Option<Decimal>,
    pub savings_ids: Vec<i64>,
}

impl PaymentDocumentDetails {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self
            .name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(DomainError::InvalidPaymentDocumentName);
        }
        validate_payment_document(
            self.amount,
            self.card_type,
            self.due_date,
            self.grace_period_days,
            self.next_payment_amount,
            self.affects_deposit_rate,
            self.min_monthly_spend,
            &self.savings_ids,
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
    #[error("Деление на ноль невозможно")]
    DivisionByZero,
    #[error("Неизвестная валюта: {0}")]
    UnknownCurrency(String),
    #[error("Код валюты должен состоять из трёх заглавных латинских букв")]
    InvalidCurrencyCode,
    #[error("Курс валюты должен быть положительным")]
    InvalidExchangeRate,
    #[error("Курс валюты должен содержать не более 12 знаков после запятой")]
    ExchangeRatePrecision,
    #[error("Не удалось точно пересчитать курс валюты")]
    ExchangeRateOverflow,
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
    #[error("Неизвестная платёжная система: {0}")]
    UnknownPaymentSystem(String),
    #[error("Неизвестный тип платёжной карты: {0}")]
    UnknownPaymentCardType(String),
    #[error("Баланс дебетовой карты не может быть отрицательным, а кредитный лимит должен быть положительным")]
    InvalidPaymentDocumentAmount,
    #[error("Название платёжной карты не может быть пустым")]
    InvalidPaymentDocumentName,
    #[error("Для кредитной карты укажите дату платежа и неотрицательный льготный период; у дебетовой карты эти поля отсутствуют")]
    InvalidPaymentCardTerms,
    #[error("Условие повышенной ставки требует положительного минимума трат и хотя бы одного связанного накопления")]
    InvalidPaymentRateCondition,
    #[error("Размер следующего платежа не может быть отрицательным")]
    InvalidNextPaymentAmount,
    #[error("Название товара или услуги не может быть пустым")]
    InvalidPriceName,
    #[error("Цена должна содержать не более 12 знаков после запятой")]
    InvalidPricePrecision,
    #[error("Начало периода не может быть позже его окончания")]
    InvalidPriceDateRange,
    #[error("Связанные накопления не должны повторяться")]
    DuplicatePaymentDocumentSavings,
    #[error("Неизвестный тип операции перевода: {0}")]
    UnknownTransferOperationType(String),
    #[error("Неизвестная сторона перевода: {0}")]
    UnknownTransferEndpoint(String),
    #[error("Неверное направление перевода для выбранного типа операции")]
    InvalidTransferDirection,
    #[error("Валюта перевода должна совпадать с валютой обоих счетов")]
    TransferCurrencyMismatch,
    #[error("Источник и получатель перевода должны быть разными счетами")]
    TransferSameAccount,
    #[error("Кредитные карты не участвуют в переводах: их лимит не является денежными средствами")]
    TransferCreditCardForbidden,
    #[error("Валюта счёта не указана; перевод возможен только при явно совпадающей валюте")]
    TransferAccountCurrencyMissing,
    #[error("Неактивное накопление нельзя выбрать для нового перевода")]
    TransferSavingsInactive,
    #[error("Комментарий перевода слишком длинный")]
    InvalidTransferComment,
}

#[expect(
    clippy::too_many_arguments,
    reason = "Extends the established payment document validation signature"
)]
pub fn validate_payment_document(
    amount: Decimal,
    card_type: PaymentCardType,
    due_date: Option<NaiveDate>,
    grace_period_days: Option<i64>,
    next_payment_amount: Option<Decimal>,
    affects_deposit_rate: bool,
    min_monthly_spend: Option<Decimal>,
    savings_ids: &[i64],
) -> Result<(), DomainError> {
    if amount < Decimal::ZERO || (card_type == PaymentCardType::Credit && amount == Decimal::ZERO) {
        return Err(DomainError::InvalidPaymentDocumentAmount);
    }
    if let Some(next_payment) = next_payment_amount {
        if next_payment < Decimal::ZERO {
            return Err(DomainError::InvalidNextPaymentAmount);
        }
    }

    match (card_type, due_date, grace_period_days) {
        (PaymentCardType::Debit, None, None) | (PaymentCardType::Credit, Some(_), Some(0..)) => {}
        _ => return Err(DomainError::InvalidPaymentCardTerms),
    }

    if affects_deposit_rate {
        if !matches!(min_monthly_spend, Some(value) if value > Decimal::ZERO)
            || savings_ids.is_empty()
        {
            return Err(DomainError::InvalidPaymentRateCondition);
        }
    } else if min_monthly_spend.is_some() || !savings_ids.is_empty() {
        return Err(DomainError::InvalidPaymentRateCondition);
    }

    let unique_savings: std::collections::HashSet<_> = savings_ids.iter().collect();
    if unique_savings.len() != savings_ids.len() {
        return Err(DomainError::DuplicatePaymentDocumentSavings);
    }
    Ok(())
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

/// Calculates a default interest amount from a balance and a selected percentage rate.
pub fn calculate_interest_amount(
    balance: Decimal,
    rate_percent: Decimal,
) -> Result<Decimal, DomainError> {
    if balance < Decimal::ZERO {
        return Err(DomainError::InvalidSavingsAmount);
    }
    if rate_percent < Decimal::ZERO {
        return Err(DomainError::InvalidSavingsRate);
    }

    let amount = balance
        .checked_mul(rate_percent)
        .and_then(|amount| amount.checked_div(Decimal::ONE_HUNDRED))
        .ok_or(DomainError::AmountOverflow)?;

    Ok(amount.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero))
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

/// Parses an arithmetic expression with positive amount operands.
///
/// Multiplication and division take precedence over addition and subtraction.
/// Each division is rounded to two decimal places using midpoint-away-from-zero.
pub fn parse_amount_expression(input: &str) -> Result<Decimal, DomainError> {
    let mut parser = AmountExpressionParser { input, position: 0 };
    let amount = parser.parse_expression()?;
    parser.skip_whitespace();
    if parser.position != input.len() {
        return Err(DomainError::InvalidAmountFormat);
    }
    if amount <= Decimal::ZERO {
        return Err(DomainError::InvalidAmount);
    }
    Ok(amount)
}

struct AmountExpressionParser<'a> {
    input: &'a str,
    position: usize,
}

impl AmountExpressionParser<'_> {
    fn parse_expression(&mut self) -> Result<Decimal, DomainError> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_whitespace();
            let operation = match self.peek() {
                Some('+') | Some('-') => self.advance(),
                _ => break,
            };
            let right = self.parse_term()?;
            value = match operation {
                Some('+') => value.checked_add(right),
                Some('-') => value.checked_sub(right),
                _ => return Err(DomainError::InvalidAmountFormat),
            }
            .ok_or(DomainError::AmountOverflow)?;
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<Decimal, DomainError> {
        let mut value = self.parse_factor()?;
        if value <= Decimal::ZERO {
            return Err(DomainError::InvalidAmount);
        }
        loop {
            self.skip_whitespace();
            let operation = match self.peek() {
                Some('*') | Some('/') => self.advance(),
                _ => break,
            };
            let right = self.parse_factor()?;
            value = match operation {
                Some('*') if right > Decimal::ZERO => value.checked_mul(right),
                Some('/') if right == Decimal::ZERO => {
                    return Err(DomainError::DivisionByZero);
                }
                Some('/') => value.checked_div(right).map(|quotient| {
                    quotient.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
                }),
                _ => return Err(DomainError::InvalidAmount),
            }
            .ok_or(DomainError::AmountOverflow)?;
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<Decimal, DomainError> {
        self.skip_whitespace();
        let start = self.position;
        while matches!(self.peek(), Some(character) if character.is_ascii_digit() || matches!(character, '.' | ','))
        {
            self.advance();
        }
        if start == self.position {
            return Err(DomainError::InvalidAmountFormat);
        }
        parse_non_negative_decimal(&self.input[start..self.position])
            .map_err(|_| DomainError::InvalidAmountFormat)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(character) if character.is_whitespace()) {
            self.advance();
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.position..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.position += character.len_utf8();
        Some(character)
    }
}

/// Formats a decimal amount in a locale-independent representation for SQLite.
pub fn canonical_amount(amount: Decimal) -> String {
    amount.normalize().to_string()
}

/// Максимальная длина комментария перевода, включая пробелы.
pub const MAX_TRANSFER_COMMENT_LENGTH: usize = 500;

/// Тип операции страницы «Переводы».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferOperationType {
    TopUp,
    Transfer,
    Withdrawal,
}

impl TransferOperationType {
    pub const ALL: [Self; 3] = [Self::TopUp, Self::Transfer, Self::Withdrawal];

    pub const fn code(self) -> &'static str {
        match self {
            Self::TopUp => "top_up",
            Self::Transfer => "transfer",
            Self::Withdrawal => "withdrawal",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::TopUp => "Пополнение",
            Self::Transfer => "Перевод",
            Self::Withdrawal => "Снятие наличных",
        }
    }

    /// Проверяет, допустимо ли направление для этого типа операции.
    pub const fn allows(self, source: &TransferEndpoint, destination: &TransferEndpoint) -> bool {
        match self {
            Self::TopUp => {
                matches!(source, TransferEndpoint::External)
                    && matches!(
                        destination,
                        TransferEndpoint::Card(_) | TransferEndpoint::Savings(_)
                    )
            }
            Self::Transfer => {
                matches!(
                    source,
                    TransferEndpoint::Card(_) | TransferEndpoint::Savings(_)
                ) && matches!(
                    destination,
                    TransferEndpoint::Card(_) | TransferEndpoint::Savings(_)
                )
            }
            Self::Withdrawal => {
                matches!(
                    source,
                    TransferEndpoint::Card(_) | TransferEndpoint::Savings(_)
                ) && matches!(destination, TransferEndpoint::Cash)
            }
        }
    }
}

impl FromStr for TransferOperationType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "top_up" => Ok(Self::TopUp),
            "transfer" => Ok(Self::Transfer),
            "withdrawal" => Ok(Self::Withdrawal),
            other => Err(DomainError::UnknownTransferOperationType(other.to_owned())),
        }
    }
}

impl std::fmt::Display for TransferOperationType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.title())
    }
}

/// Сторона перевода. `External` и `Cash` не являются учитываемыми счетами.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferEndpoint {
    External,
    Cash,
    Card(i64),
    Savings(i64),
}

impl TransferEndpoint {
    pub const fn kind_code(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Cash => "cash",
            Self::Card(_) => "card",
            Self::Savings(_) => "savings",
        }
    }

    pub const fn payment_document_id(self) -> Option<i64> {
        match self {
            Self::Card(id) => Some(id),
            _ => None,
        }
    }

    pub const fn savings_id(self) -> Option<i64> {
        match self {
            Self::Savings(id) => Some(id),
            _ => None,
        }
    }

    /// Собирает сторону из кода и идентификаторов, прочитанных из SQL.
    pub fn from_code(
        kind: &str,
        card_id: Option<i64>,
        savings_id: Option<i64>,
    ) -> Result<Self, DomainError> {
        match kind {
            "external" => Ok(Self::External),
            "cash" => Ok(Self::Cash),
            "card" => card_id
                .map(Self::Card)
                .ok_or_else(|| DomainError::UnknownTransferEndpoint(kind.to_owned())),
            "savings" => savings_id
                .map(Self::Savings)
                .ok_or_else(|| DomainError::UnknownTransferEndpoint(kind.to_owned())),
            other => Err(DomainError::UnknownTransferEndpoint(other.to_owned())),
        }
    }
}

impl FromStr for TransferEndpoint {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (kind, id) = value.split_once(':').unwrap_or((value, ""));
        match kind {
            "external" => Ok(Self::External),
            "cash" => Ok(Self::Cash),
            "card" | "savings" => {
                let id = id
                    .parse::<i64>()
                    .map_err(|_| DomainError::UnknownTransferEndpoint(value.to_owned()))?;
                if id <= 0 {
                    return Err(DomainError::UnknownTransferEndpoint(value.to_owned()));
                }
                if kind == "card" {
                    Ok(Self::Card(id))
                } else {
                    Ok(Self::Savings(id))
                }
            }
            other => Err(DomainError::UnknownTransferEndpoint(other.to_owned())),
        }
    }
}

impl std::fmt::Display for TransferEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::External => formatter.write_str("external"),
            Self::Cash => formatter.write_str("cash"),
            Self::Card(id) => write!(formatter, "card:{id}"),
            Self::Savings(id) => write!(formatter, "savings:{id}"),
        }
    }
}

/// Проверяет валюту учитываемого счёта перед переводом.
pub fn validate_transfer_account_currency(
    account_currency: Option<Currency>,
    operation_currency: Currency,
) -> Result<(), DomainError> {
    match account_currency {
        None => Err(DomainError::TransferAccountCurrencyMissing),
        Some(currency) if currency != operation_currency => {
            Err(DomainError::TransferCurrencyMismatch)
        }
        Some(_) => Ok(()),
    }
}

/// Кредитные карты не могут участвовать в переводах.
pub fn validate_transfer_card_type(card_type: PaymentCardType) -> Result<(), DomainError> {
    match card_type {
        PaymentCardType::Debit => Ok(()),
        PaymentCardType::Credit => Err(DomainError::TransferCreditCardForbidden),
    }
}

/// Неактивные накопления сохраняются в истории, но недоступны для новых операций.
pub fn validate_transfer_savings_active(is_active: bool) -> Result<(), DomainError> {
    if is_active {
        Ok(())
    } else {
        Err(DomainError::TransferSavingsInactive)
    }
}

/// Параметры операции «Переводы» без технического идентификатора записи.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDetails {
    pub operation_date: NaiveDate,
    pub operation_type: TransferOperationType,
    pub source: TransferEndpoint,
    pub destination: TransferEndpoint,
    pub amount: Decimal,
    pub currency: Currency,
    pub comment: Option<String>,
}

impl TransferDetails {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_transfer(
            self.operation_type,
            &self.source,
            &self.destination,
            self.amount,
            self.comment.as_deref(),
        )
    }
}

/// Запись журнала переводов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRecord {
    pub id: i64,
    pub operation_date: NaiveDate,
    pub operation_type: TransferOperationType,
    pub source: TransferEndpoint,
    pub destination: TransferEndpoint,
    pub amount: Decimal,
    pub currency: Currency,
    pub comment: Option<String>,
}

/// Чистая валидация операции; проверки, требующие БД, выполняются в `finance-db`.
pub fn validate_transfer(
    operation_type: TransferOperationType,
    source: &TransferEndpoint,
    destination: &TransferEndpoint,
    amount: Decimal,
    comment: Option<&str>,
) -> Result<(), DomainError> {
    if amount <= Decimal::ZERO {
        return Err(DomainError::InvalidAmount);
    }
    if !operation_type.allows(source, destination) {
        return Err(DomainError::InvalidTransferDirection);
    }
    if source == destination {
        return Err(DomainError::TransferSameAccount);
    }
    if comment.is_some_and(|text| text.chars().count() > MAX_TRANSFER_COMMENT_LENGTH) {
        return Err(DomainError::InvalidTransferComment);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::{
        calculate_interest_amount, canonical_amount, parse_amount, parse_amount_expression,
        parse_non_negative_decimal, rebase_exchange_rate, validate_currency_code,
        validate_exchange_rate, validate_payment_document, validate_price, validate_savings,
        validate_transfer_account_currency, validate_transfer_card_type,
        validate_transfer_savings_active, Currency, DomainError, InterestDestination,
        PaymentCardType, PaymentDocumentDetails, PaymentSystem, PriceFilter, RegularExpenseFilter,
        SavingsTerm, TransferDetails, TransferEndpoint, TransferOperationType,
        MAX_TRANSFER_COMMENT_LENGTH,
    };

    #[test]
    fn price_validation_requires_name_positive_amount_and_supported_precision() {
        assert!(validate_price("  Молоко  ", Decimal::new(125, 2)).is_ok());
        assert_eq!(
            validate_price(" ", Decimal::ONE),
            Err(DomainError::InvalidPriceName)
        );
        assert_eq!(
            validate_price("Товар", Decimal::ZERO),
            Err(DomainError::InvalidAmount)
        );
        assert_eq!(
            validate_price("Товар", Decimal::new(1, 13)),
            Err(DomainError::InvalidPricePrecision)
        );
    }

    #[test]
    fn price_filter_rejects_reversed_date_range() {
        let filter = PriceFilter {
            date_from: NaiveDate::from_ymd_opt(2026, 2, 1),
            date_to: NaiveDate::from_ymd_opt(2026, 1, 31),
            ..PriceFilter::default()
        };

        assert_eq!(filter.validate(), Err(DomainError::InvalidPriceDateRange));
    }

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
    fn calculate_interest_amount_applies_base_rate_to_balance() {
        assert_eq!(
            calculate_interest_amount(Decimal::from(1_000), Decimal::new(55, 1)),
            Ok(Decimal::from(55))
        );
    }

    #[test]
    fn calculate_interest_amount_applies_higher_rate_to_balance() {
        assert_eq!(
            calculate_interest_amount(Decimal::from(1_000), Decimal::new(725, 2)),
            Ok(Decimal::new(725, 1))
        );
    }

    #[test]
    fn calculate_interest_amount_rounds_only_the_final_value_to_hundredths() {
        assert_eq!(
            calculate_interest_amount(Decimal::new(1_111, 3), Decimal::from(3)),
            Ok(Decimal::new(3, 2))
        );
    }

    #[test]
    fn calculate_interest_amount_rounds_half_a_kopeck_away_from_zero() {
        assert_eq!(
            calculate_interest_amount(Decimal::ONE, Decimal::new(5, 1)),
            Ok(Decimal::new(1, 2))
        );
    }

    #[test]
    fn calculate_interest_amount_rejects_negative_balance_or_rate() {
        assert_eq!(
            calculate_interest_amount(Decimal::NEGATIVE_ONE, Decimal::ONE),
            Err(DomainError::InvalidSavingsAmount)
        );
        assert_eq!(
            calculate_interest_amount(Decimal::ONE, Decimal::NEGATIVE_ONE),
            Err(DomainError::InvalidSavingsRate)
        );
    }

    #[test]
    fn calculate_interest_amount_reports_overflow() {
        assert_eq!(
            calculate_interest_amount(Decimal::MAX, Decimal::from(2)),
            Err(DomainError::AmountOverflow)
        );
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
    fn parse_amount_expression_subtracts_positive_amounts() {
        assert_eq!(parse_amount_expression("100 - 23"), Ok(Decimal::from(77)));
        assert_eq!(parse_amount_expression("1 - 2 + 3"), Ok(Decimal::from(2)));
    }

    #[test]
    fn parse_amount_expression_multiplies_with_precedence() {
        assert_eq!(parse_amount_expression("10 + 2 * 3"), Ok(Decimal::from(16)));
    }

    #[test]
    fn parse_amount_expression_divides_and_rounds_to_two_decimal_places() {
        assert_eq!(parse_amount_expression("1 / 8"), Ok(Decimal::new(13, 2)));
        assert_eq!(parse_amount_expression("10 / 3"), Ok(Decimal::new(333, 2)));
    }

    #[test]
    fn parse_amount_expression_rounds_each_division_before_the_next_operation() {
        assert_eq!(
            parse_amount_expression("20 / 3 * 3"),
            Ok(Decimal::new(2001, 2))
        );
    }

    #[test]
    fn parse_amount_expression_rejects_non_positive_results() {
        assert_eq!(
            parse_amount_expression("1 - 2"),
            Err(DomainError::InvalidAmount)
        );
        assert_eq!(
            parse_amount_expression("1 / 1000"),
            Err(DomainError::InvalidAmount)
        );
    }

    #[test]
    fn parse_amount_expression_rejects_division_by_zero() {
        assert_eq!(
            parse_amount_expression("10 / 0"),
            Err(DomainError::DivisionByZero)
        );
    }

    #[test]
    fn parse_amount_expression_rejects_empty_terms_and_other_operators() {
        assert_eq!(
            parse_amount_expression("1 + + 2"),
            Err(DomainError::InvalidAmountFormat)
        );
        assert_eq!(
            parse_amount_expression("1 ** 2"),
            Err(DomainError::InvalidAmountFormat)
        );
    }

    #[test]
    fn parse_amount_expression_reports_overflow() {
        assert_eq!(
            parse_amount_expression("79228162514264337593543950335 + 1"),
            Err(DomainError::AmountOverflow)
        );
        assert_eq!(
            parse_amount_expression("79228162514264337593543950335 * 2"),
            Err(DomainError::AmountOverflow)
        );
    }

    #[test]
    fn canonical_amount_removes_unneeded_scale() {
        assert_eq!(canonical_amount(Decimal::new(1200, 2)), "12");
    }

    #[test]
    fn rebase_exchange_rate_uses_new_primary_rate() {
        let rate = rebase_exchange_rate(Decimal::from(90), Decimal::from(120));

        assert_eq!(rate, Ok(Decimal::new(75, 2)));
    }

    #[test]
    fn rebase_exchange_rate_rounds_non_terminating_decimal_to_twenty_four_places() {
        let rate = rebase_exchange_rate(Decimal::ONE, Decimal::from(3));

        assert_eq!(
            rate,
            Ok(Decimal::from_str("0.333333333333333333333333").unwrap())
        );
    }

    #[test]
    fn validate_exchange_rate_rejects_non_positive_values() {
        assert_eq!(
            validate_exchange_rate(Decimal::ZERO),
            Err(DomainError::InvalidExchangeRate)
        );
    }

    #[test]
    fn validate_currency_code_accepts_three_uppercase_ascii_letters() {
        assert_eq!(validate_currency_code("EUR"), Ok(()));
    }

    #[test]
    fn validate_currency_code_rejects_lowercase_and_wrong_length() {
        assert_eq!(
            validate_currency_code("eur"),
            Err(DomainError::InvalidCurrencyCode)
        );
        assert_eq!(
            validate_currency_code("US"),
            Err(DomainError::InvalidCurrencyCode)
        );
    }

    #[test]
    fn validate_exchange_rate_rejects_more_than_twelve_fractional_digits() {
        assert_eq!(
            validate_exchange_rate(Decimal::from_str("1.0000000000001").unwrap()),
            Err(DomainError::ExchangeRatePrecision)
        );
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

    #[test]
    fn other_interest_destination_uses_its_storage_code_and_title() {
        assert_eq!(InterestDestination::Other.code(), "other");
        assert_eq!(InterestDestination::Other.title(), "Другое");
    }

    #[test]
    fn all_interest_destinations_include_other() {
        assert_eq!(
            InterestDestination::ALL,
            [
                InterestDestination::Reinvest,
                InterestDestination::Withdraw,
                InterestDestination::Other,
            ]
        );
    }

    #[test]
    fn interest_destination_parser_accepts_other_and_rejects_unknown_codes() {
        assert_eq!(
            "other".parse::<InterestDestination>(),
            Ok(InterestDestination::Other)
        );
        assert_eq!(
            "unknown".parse::<InterestDestination>(),
            Err(DomainError::UnknownInterestDestination(
                "unknown".to_owned()
            ))
        );
    }

    #[test]
    fn payment_system_storage_preserves_contract_values() {
        assert_eq!(PaymentSystem::Visa.code(), "VISA");
        assert_eq!(
            "MasterCard".parse::<PaymentSystem>(),
            Ok(PaymentSystem::MasterCard)
        );
        assert_eq!(
            "MASTERCARD".parse::<PaymentSystem>(),
            Err(DomainError::UnknownPaymentSystem("MASTERCARD".to_owned()))
        );
    }

    #[test]
    fn payment_document_details_rejects_empty_optional_name() {
        let details = PaymentDocumentDetails {
            name: Some("  ".to_owned()),
            bank_id: 1,
            payment_system: PaymentSystem::Visa,
            card_type: PaymentCardType::Debit,
            amount: Decimal::ONE,
            currency: None,
            due_date: None,
            grace_period_days: None,
            next_payment_amount: None,
            affects_deposit_rate: false,
            min_monthly_spend: None,
            savings_ids: Vec::new(),
        };

        assert_eq!(
            details.validate(),
            Err(DomainError::InvalidPaymentDocumentName)
        );
    }

    #[test]
    fn validate_payment_document_accepts_debit_without_credit_fields() {
        assert_eq!(
            validate_payment_document(
                Decimal::ONE,
                PaymentCardType::Debit,
                None,
                None,
                None,
                false,
                None,
                &[],
            ),
            Ok(())
        );
    }

    #[test]
    fn validate_payment_document_accepts_credit_and_rate_condition() {
        assert_eq!(
            validate_payment_document(
                Decimal::ONE,
                PaymentCardType::Credit,
                NaiveDate::from_ymd_opt(2026, 12, 1),
                Some(0),
                Some(Decimal::ONE),
                true,
                Some(Decimal::ONE),
                &[4],
            ),
            Ok(())
        );
    }

    #[test]
    fn validate_payment_document_rejects_debit_with_credit_fields() {
        assert_eq!(
            validate_payment_document(
                Decimal::ONE,
                PaymentCardType::Debit,
                NaiveDate::from_ymd_opt(2026, 12, 1),
                Some(0),
                None,
                false,
                None,
                &[],
            ),
            Err(DomainError::InvalidPaymentCardTerms)
        );
    }

    #[test]
    fn validate_payment_document_rejects_rate_condition_without_savings() {
        assert_eq!(
            validate_payment_document(
                Decimal::ONE,
                PaymentCardType::Debit,
                None,
                None,
                None,
                true,
                Some(Decimal::ONE),
                &[],
            ),
            Err(DomainError::InvalidPaymentRateCondition)
        );
    }

    #[test]
    fn validate_payment_document_rejects_duplicate_savings_links() {
        assert_eq!(
            validate_payment_document(
                Decimal::ONE,
                PaymentCardType::Debit,
                None,
                None,
                None,
                true,
                Some(Decimal::ONE),
                &[4, 4],
            ),
            Err(DomainError::DuplicatePaymentDocumentSavings)
        );
    }

    #[test]
    fn transfer_operation_codes_roundtrip() {
        for operation in TransferOperationType::ALL {
            assert_eq!(operation.to_string(), operation.title());
            assert_eq!(
                operation.code().parse::<TransferOperationType>(),
                Ok(operation)
            );
        }
        assert_eq!(
            "other".parse::<TransferOperationType>(),
            Err(DomainError::UnknownTransferOperationType(
                "other".to_owned()
            ))
        );
    }

    #[test]
    fn transfer_endpoint_codes_roundtrip() {
        let endpoints = [
            TransferEndpoint::External,
            TransferEndpoint::Cash,
            TransferEndpoint::Card(12),
            TransferEndpoint::Savings(34),
        ];
        for endpoint in endpoints {
            let code = endpoint.to_string();
            assert_eq!(code.parse::<TransferEndpoint>(), Ok(endpoint));
            assert_eq!(
                TransferEndpoint::from_code(
                    endpoint.kind_code(),
                    endpoint.payment_document_id(),
                    endpoint.savings_id()
                ),
                Ok(endpoint)
            );
        }
        assert_eq!(
            "card:0".parse::<TransferEndpoint>(),
            Err(DomainError::UnknownTransferEndpoint("card:0".to_owned()))
        );
    }

    #[test]
    fn transfer_validation_rejects_invalid_amount_direction_and_self_transfer() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        let valid = TransferDetails {
            operation_date: date,
            operation_type: TransferOperationType::Transfer,
            source: TransferEndpoint::Card(1),
            destination: TransferEndpoint::Savings(2),
            amount: Decimal::new(125, 2),
            currency: Currency::Rub,
            comment: None,
        };
        assert_eq!(valid.validate(), Ok(()));

        let mut invalid = valid.clone();
        invalid.amount = Decimal::ZERO;
        assert_eq!(invalid.validate(), Err(DomainError::InvalidAmount));

        invalid = valid.clone();
        invalid.destination = TransferEndpoint::External;
        assert_eq!(
            invalid.validate(),
            Err(DomainError::InvalidTransferDirection)
        );

        invalid = valid.clone();
        invalid.destination = TransferEndpoint::Card(1);
        assert_eq!(invalid.validate(), Err(DomainError::TransferSameAccount));

        invalid = valid.clone();
        invalid.comment = Some("x".repeat(MAX_TRANSFER_COMMENT_LENGTH + 1));
        assert_eq!(invalid.validate(), Err(DomainError::InvalidTransferComment));
    }

    #[test]
    fn transfer_account_validation_requires_explicit_and_matching_currency() {
        assert_eq!(
            validate_transfer_account_currency(None, Currency::Rub),
            Err(DomainError::TransferAccountCurrencyMissing)
        );
        assert_eq!(
            validate_transfer_account_currency(Some(Currency::Usd), Currency::Rub),
            Err(DomainError::TransferCurrencyMismatch)
        );
        assert_eq!(
            validate_transfer_account_currency(Some(Currency::Rub), Currency::Rub),
            Ok(())
        );
        assert_eq!(
            validate_transfer_card_type(PaymentCardType::Credit),
            Err(DomainError::TransferCreditCardForbidden)
        );
        assert_eq!(validate_transfer_card_type(PaymentCardType::Debit), Ok(()));
        assert_eq!(
            validate_transfer_savings_active(false),
            Err(DomainError::TransferSavingsInactive)
        );
        assert_eq!(validate_transfer_savings_active(true), Ok(()));
    }

    #[test]
    fn transfer_validation_accepts_only_declared_directions() {
        assert!(TransferOperationType::TopUp
            .allows(&TransferEndpoint::External, &TransferEndpoint::Card(1)));
        assert!(!TransferOperationType::TopUp
            .allows(&TransferEndpoint::Card(1), &TransferEndpoint::Cash));
        assert!(TransferOperationType::Withdrawal
            .allows(&TransferEndpoint::Savings(1), &TransferEndpoint::Cash));
        assert!(!TransferOperationType::Withdrawal
            .allows(&TransferEndpoint::Savings(1), &TransferEndpoint::Card(2)));
    }

    #[test]
    fn regular_expense_filter_matches_only_selected_categories() {
        assert_eq!(RegularExpenseFilter::default(), RegularExpenseFilter::All);
        assert!(!RegularExpenseFilter::All.is_active());
        assert!(RegularExpenseFilter::Regular.is_active());
        assert!(RegularExpenseFilter::NotRegular.is_active());
        assert!(RegularExpenseFilter::All.matches(true));
        assert!(RegularExpenseFilter::All.matches(false));
        assert!(RegularExpenseFilter::Regular.matches(true));
        assert!(!RegularExpenseFilter::Regular.matches(false));
        assert!(!RegularExpenseFilter::NotRegular.matches(true));
        assert!(RegularExpenseFilter::NotRegular.matches(false));
    }
}
