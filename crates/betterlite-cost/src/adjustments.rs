use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{
    AdjustmentField, DiscountEntry, MarginEntry, MarginFieldEntry,
};
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
    #[error("cost adjustment field {field} cannot convert to a number")]
    InvalidField { field: AdjustmentField },
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

const fn margin_field_amount(
    entry: MarginFieldEntry,
    field: AdjustmentField,
) -> Result<Decimal, AdjustmentError> {
    match entry {
        MarginFieldEntry::Absent => Ok(Decimal::ZERO),
        MarginFieldEntry::Number(value) => Ok(value),
        MarginFieldEntry::Invalid => Err(AdjustmentError::InvalidField { field }),
    }
}

const fn checked(
    result: Option<Decimal>,
    operation: AdjustmentOperation,
) -> Result<Decimal, AdjustmentError> {
    match result {
        Some(value) => Ok(value),
        None => Err(AdjustmentError::Overflow { operation }),
    }
}

fn margin_percent_and_fixed(
    entry: Option<&MarginEntry>,
) -> Result<(Decimal, Decimal), AdjustmentError> {
    match entry {
        Some(MarginEntry::Percent(percent)) => Ok((*percent, Decimal::ZERO)),
        Some(MarginEntry::Invalid) => Err(AdjustmentError::InvalidField {
            field: AdjustmentField::MarginPercent,
        }),
        Some(MarginEntry::Split {
            percentage,
            fixed_amount,
        }) => Ok((
            margin_field_amount(*percentage, AdjustmentField::Percentage)?,
            margin_field_amount(*fixed_amount, AdjustmentField::FixedAmount)?,
        )),
        Some(MarginEntry::Unsupported) | None => Ok((Decimal::ZERO, Decimal::ZERO)),
    }
}

// a discount entry carries the same scalar percent a margin entry can, so relabel margin_percent_and_fixed's result onto the discount side
fn discount_percent(entry: Option<&DiscountEntry>) -> Result<Decimal, AdjustmentError> {
    let as_margin = match entry {
        Some(DiscountEntry::Percent(percent)) => Some(MarginEntry::Percent(*percent)),
        Some(DiscountEntry::Invalid) => Some(MarginEntry::Invalid),
        Some(DiscountEntry::Unsupported) | None => None,
    };
    match margin_percent_and_fixed(as_margin.as_ref()) {
        Ok((percent, _)) => Ok(percent),
        Err(AdjustmentError::InvalidField { .. }) => Err(AdjustmentError::InvalidField {
            field: AdjustmentField::DiscountPercent,
        }),
        Err(overflow @ AdjustmentError::Overflow { .. }) => Err(overflow),
    }
}

pub fn apply_cost_discount(
    base_cost: Decimal,
    provider: Option<&str>,
    config: &BTreeMap<String, DiscountEntry>,
) -> Result<DiscountResult, AdjustmentError> {
    let discount_percent = discount_percent(selected_entry(provider, config, None))?;
    let Some(saved_amount) = base_cost.checked_mul(discount_percent) else {
        return Err(AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseTimesDiscount,
        });
    };
    let Some(final_cost) = base_cost.checked_sub(saved_amount) else {
        return Err(AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseMinusSaved,
        });
    };
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
        margin_percent_and_fixed(selected_entry(provider, config, Some("global")))?;
    let percentage_amount = checked(
        base_cost.checked_mul(margin_percent),
        AdjustmentOperation::BaseTimesPercentage,
    )?;
    let total_amount = checked(
        percentage_amount.checked_add(fixed_amount),
        AdjustmentOperation::PercentagePlusFixed,
    )?;
    let final_cost = checked(
        base_cost.checked_add(total_amount),
        AdjustmentOperation::BasePlusMargin,
    )?;
    Ok(MarginResult {
        final_cost,
        margin_percent,
        fixed_amount,
        total_amount,
    })
}

mod tests;
