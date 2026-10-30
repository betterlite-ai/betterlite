use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_config::cost_adjustments::{CostAdjustmentMap, CostAdjustmentValue};
use rust_decimal::Decimal;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdjustmentError {
    #[error("cost adjustment overflow while calculating {operation}")]
    Overflow { operation: &'static str },
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

pub fn apply_cost_discount(
    base_cost: Decimal,
    provider: Option<&str>,
    config: &CostAdjustmentMap,
) -> Result<DiscountResult, AdjustmentError> {
    let discount_percent = provider
        .filter(|name| !name.is_empty())
        .and_then(|name| config.get(name))
        .and_then(decimal_number)
        .unwrap_or(Decimal::ZERO);
    let (final_cost, saved_amount) = discount_values(base_cost, discount_percent)?;
    Ok(DiscountResult {
        final_cost,
        discount_percent,
        saved_amount,
    })
}

fn discount_values(
    base_cost: Decimal,
    discount_percent: Decimal,
) -> Result<(Decimal, Decimal), AdjustmentError> {
    let saved_amount =
        base_cost
            .checked_mul(discount_percent)
            .ok_or(AdjustmentError::Overflow {
                operation: "base * discount",
            })?;
    let final_cost = base_cost
        .checked_sub(saved_amount)
        .ok_or(AdjustmentError::Overflow {
            operation: "base - saved",
        })?;
    Ok((final_cost, saved_amount))
}

pub fn apply_cost_margin(
    base_cost: Decimal,
    provider: Option<&str>,
    config: &CostAdjustmentMap,
) -> Result<MarginResult, AdjustmentError> {
    let Some(value) = provider
        .filter(|name| !name.is_empty())
        .and_then(|name| config.get(name))
        .or_else(|| config.get("global"))
    else {
        return Ok(MarginResult {
            final_cost: base_cost,
            margin_percent: Decimal::ZERO,
            fixed_amount: Decimal::ZERO,
            total_amount: Decimal::ZERO,
        });
    };
    let (margin_percent, fixed_amount) = margin_components(value);
    let percentage_amount =
        base_cost
            .checked_mul(margin_percent)
            .ok_or(AdjustmentError::Overflow {
                operation: "base * percentage",
            })?;
    let total_amount =
        percentage_amount
            .checked_add(fixed_amount)
            .ok_or(AdjustmentError::Overflow {
                operation: "percentage + fixed",
            })?;
    let final_cost = base_cost
        .checked_add(total_amount)
        .ok_or(AdjustmentError::Overflow {
            operation: "base + margin",
        })?;
    Ok(MarginResult {
        final_cost,
        margin_percent,
        fixed_amount,
        total_amount,
    })
}

fn margin_components(value: &CostAdjustmentValue) -> (Decimal, Decimal) {
    match value {
        CostAdjustmentValue::Number(_) => (
            decimal_number(value).unwrap_or(Decimal::ZERO),
            Decimal::ZERO,
        ),
        CostAdjustmentValue::Object(entries) => (
            object_decimal(entries, "percentage"),
            object_decimal(entries, "fixed_amount"),
        ),
        CostAdjustmentValue::Null
        | CostAdjustmentValue::Boolean(_)
        | CostAdjustmentValue::Text(_)
        | CostAdjustmentValue::Array(_) => (Decimal::ZERO, Decimal::ZERO),
    }
}

fn decimal_number(value: &CostAdjustmentValue) -> Option<Decimal> {
    if let CostAdjustmentValue::Number(number) = value {
        Decimal::from_str(&number.to_string()).ok()
    } else {
        None
    }
}

fn object_decimal(entries: &BTreeMap<String, CostAdjustmentValue>, key: &str) -> Decimal {
    match entries.get(key) {
        Some(CostAdjustmentValue::Number(number)) => {
            Decimal::from_str(&number.to_string()).unwrap_or(Decimal::ZERO)
        }
        Some(CostAdjustmentValue::Text(text)) => Decimal::from_str(text).unwrap_or(Decimal::ZERO),
        Some(
            CostAdjustmentValue::Null
            | CostAdjustmentValue::Boolean(_)
            | CostAdjustmentValue::Object(_)
            | CostAdjustmentValue::Array(_),
        )
        | None => Decimal::ZERO,
    }
}

mod tests;
