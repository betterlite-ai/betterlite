use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{DiscountEntry, MarginEntry, MarginFieldEntry};
use rust_decimal::Decimal;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum AdjustmentOperation {
    #[strum(serialize = "base * discount")]
    BaseTimesDiscount,
    #[strum(serialize = "base - saved")]
    BaseMinusSaved,
    #[strum(serialize = "base * percentage")]
    BaseTimesPercentage,
    #[strum(serialize = "percentage + fixed")]
    PercentagePlusFixed,
    #[strum(serialize = "base + margin")]
    BasePlusMargin,
}

impl AdjustmentOperation {
    fn apply(self, left: Decimal, right: Decimal) -> Result<Decimal, AdjustmentError> {
        let result = match self {
            Self::BaseTimesDiscount | Self::BaseTimesPercentage => left.checked_mul(right),
            Self::BaseMinusSaved => left.checked_sub(right),
            Self::PercentagePlusFixed | Self::BasePlusMargin => left.checked_add(right),
        };
        result.ok_or(AdjustmentError::Overflow { operation: self })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AdjustmentError {
    #[error("cost adjustment overflow while calculating {operation}")]
    Overflow { operation: AdjustmentOperation },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscountResult {
    pub final_cost: Decimal,
    pub discount_percent: Decimal,
    pub saved_amount: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarginResult {
    pub final_cost: Decimal,
    pub margin_percent: Decimal,
    pub fixed_amount: Decimal,
    pub total_amount: Decimal,
}

fn selected_entry<'config, T>(
    provider: Option<&str>,
    config: &'config BTreeMap<String, T>,
    fallback_key: Option<&str>,
) -> Option<&'config T> {
    let selected = provider
        .filter(|name| !name.is_empty())
        .and_then(|name| config.get(name));
    match (selected, fallback_key) {
        (Some(value), _) => Some(value),
        (None, Some(key)) => config.get(key),
        (None, None) => None,
    }
}

const fn margin_field_amount(entry: MarginFieldEntry) -> Decimal {
    match entry {
        MarginFieldEntry::Absent => Decimal::ZERO,
        MarginFieldEntry::Number(value) => value,
    }
}

const fn margin_percent_and_fixed(entry: Option<&MarginEntry>) -> (Decimal, Decimal) {
    match entry {
        Some(MarginEntry::Percent(percent)) => (*percent, Decimal::ZERO),
        Some(MarginEntry::Split {
            percentage,
            fixed_amount,
        }) => (
            margin_field_amount(*percentage),
            margin_field_amount(*fixed_amount),
        ),
        Some(MarginEntry::Unsupported) | None => (Decimal::ZERO, Decimal::ZERO),
    }
}

const fn discount_percent(entry: Option<&DiscountEntry>) -> Decimal {
    match entry {
        Some(DiscountEntry::Percent(percent)) => *percent,
        Some(DiscountEntry::Unsupported) | None => Decimal::ZERO,
    }
}

pub fn apply_cost_discount(
    base_cost: Decimal,
    provider: Option<&str>,
    config: &BTreeMap<String, DiscountEntry>,
) -> Result<DiscountResult, AdjustmentError> {
    let discount_percent = discount_percent(selected_entry(provider, config, None));
    let saved_amount = AdjustmentOperation::BaseTimesDiscount.apply(base_cost, discount_percent)?;
    let final_cost = AdjustmentOperation::BaseMinusSaved.apply(base_cost, saved_amount)?;
    Ok(DiscountResult {
        final_cost,
        discount_percent,
        saved_amount,
    })
}

pub fn apply_cost_margin(
    base_cost: Decimal,
    provider: Option<&str>,
    config: &BTreeMap<String, MarginEntry>,
) -> Result<MarginResult, AdjustmentError> {
    let (margin_percent, fixed_amount) =
        margin_percent_and_fixed(selected_entry(provider, config, Some("global")));
    let percentage_amount =
        AdjustmentOperation::BaseTimesPercentage.apply(base_cost, margin_percent)?;
    let total_amount =
        AdjustmentOperation::PercentagePlusFixed.apply(percentage_amount, fixed_amount)?;
    let final_cost = AdjustmentOperation::BasePlusMargin.apply(base_cost, total_amount)?;
    Ok(MarginResult {
        final_cost,
        margin_percent,
        fixed_amount,
        total_amount,
    })
}

mod tests;
