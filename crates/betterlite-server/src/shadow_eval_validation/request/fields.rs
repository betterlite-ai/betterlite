use std::collections::BTreeSet;
use std::str::FromStr as _;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::Number;

use crate::shadow_eval_validation::error::ShadowEvalRequestError;
use crate::shadow_eval_validation::request::ShadowEvalDirection;

const DEFAULT_DURATION_DAYS: u32 = 7;

fn integer(number: &Number, field: &'static str) -> Result<i64, ShadowEvalRequestError> {
    let value = decimal(number, field)?;
    if !value.fract().is_zero() {
        return Err(validation(field, format!("{field} must be a whole number")));
    }
    value
        .to_i64()
        .ok_or_else(|| validation(field, format!("{field} is out of range")))
}

// the exact decimal a JSON number spells, including the scientific notation serde_json keeps verbatim
fn decimal(number: &Number, field: &'static str) -> Result<Decimal, ShadowEvalRequestError> {
    let token = number.to_string();
    Decimal::from_str(&token)
        .or_else(|_source| Decimal::from_scientific(&token))
        .map_err(|source| validation(field, format!("{field} must be a decimal number: {source}")))
}

// LiteLLM dedupes with dict.fromkeys, so the first spelling of each value wins
pub(super) fn deduplicated(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(super) fn validation(field: &'static str, message: String) -> ShadowEvalRequestError {
    ShadowEvalRequestError::Validation { field, message }
}

// The request model's fields as they arrive: absent or null is None, and every other value must fit the declared type.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawRequest {
    pub(super) api_key_ids: Option<Vec<String>>,
    pub(super) team_ids: Option<Vec<String>>,
    pub(super) user_ids: Option<Vec<String>>,
    pub(super) models: Option<Vec<String>>,
    pub(super) router_name: Option<String>,
    pub(super) router_names: Option<Vec<String>>,
    pub(super) direction: Option<ShadowEvalDirection>,
    pub(super) baseline_model: Option<String>,
    pub(super) shadow_percentage: Option<Number>,
    pub(super) judge_model: Option<String>,
    pub(super) duration_days: Option<Number>,
    pub(super) max_budget: Option<Number>,
}

pub(super) fn bounded_list(
    values: Option<Vec<String>>,
    field: &'static str,
    max: usize,
) -> Result<Vec<String>, ShadowEvalRequestError> {
    let values = values.unwrap_or_default();
    if values.len() > max {
        return Err(validation(
            field,
            format!("{field} must have at most {max} entries"),
        ));
    }
    Ok(deduplicated(values))
}

pub(super) fn percentage(value: Option<Number>) -> Result<f64, ShadowEvalRequestError> {
    let Some(number) = value else {
        return Err(validation(
            "shadow_percentage",
            "shadow_percentage is required".to_owned(),
        ));
    };
    let value = number.as_f64().ok_or_else(|| {
        validation(
            "shadow_percentage",
            "shadow_percentage must be a finite number".to_owned(),
        )
    })?;
    if !(0.1_f64..=100.0_f64).contains(&value) {
        return Err(validation(
            "shadow_percentage",
            "shadow_percentage must be between 0.1 and 100".to_owned(),
        ));
    }
    // Python's round(value, 2) is the correctly rounded two-decimal value of the same double
    format!("{value:.2}").parse::<f64>().map_err(|source| {
        validation(
            "shadow_percentage",
            format!("shadow_percentage cannot be rounded: {source}"),
        )
    })
}

pub(super) fn duration_days(value: Option<Number>) -> Result<u32, ShadowEvalRequestError> {
    let Some(number) = value else {
        return Ok(DEFAULT_DURATION_DAYS);
    };
    let days = integer(&number, "duration_days")?;
    if !(1_i64..=30_i64).contains(&days) {
        return Err(validation(
            "duration_days",
            "duration_days must be between 1 and 30".to_owned(),
        ));
    }
    u32::try_from(days).map_err(|source| {
        validation(
            "duration_days",
            format!("duration_days is out of range: {source}"),
        )
    })
}

pub(super) fn budget(value: Option<Number>) -> Result<Decimal, ShadowEvalRequestError> {
    let Some(number) = value else {
        return Ok(Decimal::new(100, 1));
    };
    let amount = decimal(&number, "max_budget")?;
    if amount < Decimal::new(1, 2) || amount > Decimal::new(10_000, 0) {
        return Err(validation(
            "max_budget",
            "max_budget must be between 0.01 and 10000".to_owned(),
        ));
    }
    Ok(amount)
}
