use std::str::FromStr;

use betterlite_config::types::CostAdjustmentMap;
use rust_decimal::Decimal;
use serde_json::Value;
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
    let Some(provider) = provider.filter(|name| !name.is_empty()) else {
        return Ok(DiscountResult {
            final_cost: base_cost,
            discount_percent: Decimal::ZERO,
            saved_amount: Decimal::ZERO,
        });
    };
    let Some(value) = config.get(provider) else {
        return Ok(DiscountResult {
            final_cost: base_cost,
            discount_percent: Decimal::ZERO,
            saved_amount: Decimal::ZERO,
        });
    };
    let Some(discount_percent) = decimal_number(value) else {
        return Ok(DiscountResult {
            final_cost: base_cost,
            discount_percent: Decimal::ZERO,
            saved_amount: Decimal::ZERO,
        });
    };
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
    Ok(DiscountResult {
        final_cost,
        discount_percent,
        saved_amount,
    })
}

pub fn apply_cost_margin(
    base_cost: Decimal,
    provider: Option<&str>,
    config: &CostAdjustmentMap,
) -> Result<MarginResult, AdjustmentError> {
    let value = provider
        .and_then(|name| config.get(name))
        .or_else(|| config.get("global"));
    let Some(value) = value else {
        return Ok(MarginResult {
            final_cost: base_cost,
            margin_percent: Decimal::ZERO,
            fixed_amount: Decimal::ZERO,
            total_amount: Decimal::ZERO,
        });
    };
    let (margin_percent, fixed_amount) = match value {
        Value::Number(_) => (
            decimal_number(value).unwrap_or(Decimal::ZERO),
            Decimal::ZERO,
        ),
        Value::Object(entries) => (
            object_decimal(entries, "percentage"),
            object_decimal(entries, "fixed_amount"),
        ),
        _ => (Decimal::ZERO, Decimal::ZERO),
    };
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

fn decimal_number(value: &Value) -> Option<Decimal> {
    match value {
        Value::Number(number) => Decimal::from_str(&number.to_string()).ok(),
        _ => None,
    }
}

fn object_decimal(entries: &serde_json::Map<String, Value>, key: &str) -> Decimal {
    let parsed = entries
        .get(key)
        .and_then(|value| match value {
            Value::Number(_) | Value::String(_) => {
                Decimal::from_str(&value.to_string().trim_matches('"')).ok()
            }
            _ => None,
        })
        .unwrap_or(Decimal::ZERO);
    parsed
}

mod tests;
