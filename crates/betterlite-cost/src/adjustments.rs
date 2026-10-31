use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{CostAdjustmentMap, CostAdjustmentValue};
use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdjustmentError {
    #[error("cost adjustment overflow while calculating {operation}")]
    Overflow { operation: &'static str },
    #[error(
        "cost adjustment field {field} has a numeric-looking value that cannot convert: {source}"
    )]
    InvalidNumericText {
        field: &'static str,
        #[source]
        source: DecimalLexemeError,
    },
}

impl PartialEq for AdjustmentError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Overflow { operation: left }, Self::Overflow { operation: right })
            | (
                Self::InvalidNumericText { field: left, .. },
                Self::InvalidNumericText { field: right, .. },
            ) => left == right,
            _ => false,
        }
    }
}

impl Eq for AdjustmentError {}

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

fn config_value<'config>(
    provider: Option<&str>,
    config: &'config CostAdjustmentMap,
    fallback_key: Option<&str>,
) -> Option<&'config CostAdjustmentValue> {
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
    config: &CostAdjustmentMap,
) -> Result<DiscountResult, AdjustmentError> {
    let discount_percent = config_value(provider, config, None)
        .map(|value| scalar_decimal(value, "discount_percent"))
        .transpose()?
        .flatten()
        .unwrap_or(Decimal::ZERO);
    let (final_cost, saved_amount) = discount_values(base_cost, discount_percent)?;
    Ok(DiscountResult {
        final_cost,
        discount_percent,
        saved_amount,
    })
}

const fn boolean_decimal(flag: bool) -> Decimal {
    if flag { Decimal::ONE } else { Decimal::ZERO }
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
    let Some(value) = config_value(provider, config, Some("global")) else {
        return Ok(MarginResult {
            final_cost: base_cost,
            margin_percent: Decimal::ZERO,
            fixed_amount: Decimal::ZERO,
            total_amount: Decimal::ZERO,
        });
    };
    let (margin_percent, fixed_amount) = margin_components(value)?;
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

fn margin_components(value: &CostAdjustmentValue) -> Result<(Decimal, Decimal), AdjustmentError> {
    match value {
        CostAdjustmentValue::Number(_) | CostAdjustmentValue::Boolean(_) => Ok((
            scalar_decimal(value, "margin_percent")?.unwrap_or(Decimal::ZERO),
            Decimal::ZERO,
        )),
        CostAdjustmentValue::Object(entries) => Ok((
            object_decimal(entries, "percentage")?,
            object_decimal(entries, "fixed_amount")?,
        )),
        CostAdjustmentValue::Null
        | CostAdjustmentValue::Text(_)
        | CostAdjustmentValue::Array(_) => Ok((Decimal::ZERO, Decimal::ZERO)),
    }
}

fn scalar_decimal(
    value: &CostAdjustmentValue,
    field: &'static str,
) -> Result<Option<Decimal>, AdjustmentError> {
    match value {
        CostAdjustmentValue::Number(number) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(|source| AdjustmentError::InvalidNumericText { field, source }),
        CostAdjustmentValue::Boolean(flag) => Ok(Some(boolean_decimal(*flag))),
        CostAdjustmentValue::Null
        | CostAdjustmentValue::Text(_)
        | CostAdjustmentValue::Object(_)
        | CostAdjustmentValue::Array(_) => Ok(None),
    }
}

fn object_decimal(
    entries: &BTreeMap<String, CostAdjustmentValue>,
    field: &'static str,
) -> Result<Decimal, AdjustmentError> {
    let Some(value) = entries.get(field) else {
        return Ok(Decimal::ZERO);
    };
    if let CostAdjustmentValue::Text(text) = value {
        return text_decimal(text, field);
    }
    Ok(scalar_decimal(value, field)?.unwrap_or(Decimal::ZERO))
}

fn text_decimal(text: &str, field: &'static str) -> Result<Decimal, AdjustmentError> {
    let trimmed = text.trim();
    match parse_decimal_lexeme(trimmed) {
        Ok(decimal) => Ok(decimal),
        Err(source) if trimmed.parse::<f64>().is_ok() => {
            Err(AdjustmentError::InvalidNumericText { field, source })
        }
        Err(_) => Ok(Decimal::ZERO),
    }
}

mod tests;
