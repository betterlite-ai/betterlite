use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{AdjustmentEntry, AdjustmentKind, MarginFieldEntry};
use rust_decimal::Decimal;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum AdjustmentOperation {
    #[strum(serialize = "base * percent")]
    BaseTimesPercent,
    #[strum(serialize = "percentage + fixed")]
    PercentagePlusFixed,
    #[strum(serialize = "base - amount")]
    BaseMinusAmount,
    #[strum(serialize = "base + amount")]
    BasePlusAmount,
}

impl AdjustmentOperation {
    fn apply(self, left: Decimal, right: Decimal) -> Result<Decimal, AdjustmentError> {
        let result = match self {
            Self::BaseTimesPercent => left.checked_mul(right),
            Self::PercentagePlusFixed | Self::BasePlusAmount => left.checked_add(right),
            Self::BaseMinusAmount => left.checked_sub(right),
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
pub struct AdjustmentResult {
    pub final_cost: Decimal,
    pub percent: Decimal,
    pub fixed_amount: Decimal,
    pub amount: Decimal,
}

fn selected_entry<'config>(
    provider: Option<&str>,
    config: &'config BTreeMap<String, AdjustmentEntry>,
    fallback_key: Option<&str>,
) -> Option<&'config AdjustmentEntry> {
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

const fn percent_and_fixed(entry: Option<&AdjustmentEntry>) -> (Decimal, Decimal) {
    match entry {
        Some(AdjustmentEntry::Percent(percent)) => (*percent, Decimal::ZERO),
        Some(AdjustmentEntry::Split {
            percentage,
            fixed_amount,
        }) => (
            margin_field_amount(*percentage),
            margin_field_amount(*fixed_amount),
        ),
        Some(AdjustmentEntry::Unsupported) | None => (Decimal::ZERO, Decimal::ZERO),
    }
}

pub fn apply_cost_adjustment(
    kind: AdjustmentKind,
    base_cost: Decimal,
    provider: Option<&str>,
    config: &BTreeMap<String, AdjustmentEntry>,
) -> Result<AdjustmentResult, AdjustmentError> {
    let fallback_key = match kind {
        AdjustmentKind::Discount => None,
        AdjustmentKind::Margin => Some("global"),
    };
    let (percent, fixed_amount) = percent_and_fixed(selected_entry(provider, config, fallback_key));
    let percentage_amount = AdjustmentOperation::BaseTimesPercent.apply(base_cost, percent)?;
    let amount = AdjustmentOperation::PercentagePlusFixed.apply(percentage_amount, fixed_amount)?;
    let final_cost = match kind {
        AdjustmentKind::Discount => {
            AdjustmentOperation::BaseMinusAmount.apply(base_cost, amount)?
        }
        AdjustmentKind::Margin => AdjustmentOperation::BasePlusAmount.apply(base_cost, amount)?,
    };
    Ok(AdjustmentResult {
        final_cost,
        percent,
        fixed_amount,
        amount,
    })
}

mod tests;
