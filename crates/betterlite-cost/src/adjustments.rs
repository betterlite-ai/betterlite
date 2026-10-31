use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{DiscountEntry, MarginEntry};
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

pub fn apply_cost_discount(
    base_cost: Decimal,
    provider: Option<&str>,
    config: &BTreeMap<String, DiscountEntry>,
) -> Result<DiscountResult, AdjustmentError> {
    let discount_percent = match selected_entry(provider, config, None) {
        Some(DiscountEntry::Percent(percent)) => *percent,
        Some(DiscountEntry::Unsupported) | None => Decimal::ZERO,
    };
    let saved_amount =
        base_cost
            .checked_mul(discount_percent)
            .ok_or(AdjustmentError::Overflow {
                operation: AdjustmentOperation::BaseTimesDiscount,
            })?;
    let final_cost = base_cost
        .checked_sub(saved_amount)
        .ok_or(AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseMinusSaved,
        })?;
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
    let (margin_percent, fixed_amount) = match selected_entry(provider, config, Some("global")) {
        Some(MarginEntry::Percent(percent)) => (*percent, Decimal::ZERO),
        Some(MarginEntry::Split {
            percentage,
            fixed_amount,
        }) => (
            percentage.unwrap_or(Decimal::ZERO),
            fixed_amount.unwrap_or(Decimal::ZERO),
        ),
        Some(MarginEntry::Unsupported) | None => (Decimal::ZERO, Decimal::ZERO),
    };
    let percentage_amount =
        base_cost
            .checked_mul(margin_percent)
            .ok_or(AdjustmentError::Overflow {
                operation: AdjustmentOperation::BaseTimesPercentage,
            })?;
    let total_amount =
        percentage_amount
            .checked_add(fixed_amount)
            .ok_or(AdjustmentError::Overflow {
                operation: AdjustmentOperation::PercentagePlusFixed,
            })?;
    let final_cost = base_cost
        .checked_add(total_amount)
        .ok_or(AdjustmentError::Overflow {
            operation: AdjustmentOperation::BasePlusMargin,
        })?;
    Ok(MarginResult {
        final_cost,
        margin_percent,
        fixed_amount,
        total_amount,
    })
}

mod tests;
