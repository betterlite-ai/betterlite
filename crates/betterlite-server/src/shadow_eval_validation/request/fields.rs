use std::collections::BTreeSet;
use std::str::FromStr as _;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::Number;

use crate::shadow_eval_validation::error::ShadowEvalRequestError::{self, Validation};
use crate::shadow_eval_validation::request::ShadowEvalDirection;

pub(super) const DEFAULT_DURATION: u32 = 7;

fn integer(number: &Number, field: &'static str) -> Result<i64, ShadowEvalRequestError> {
    let value = decimal(number, field)?;
    if !value.fract().is_zero() {
        return Err(Validation {
            field,
            message: format!("{field} must be a whole number"),
        });
    }
    value.to_i64().ok_or_else(|| Validation {
        field,
        message: format!("{field} is out of range"),
    })
}

// the exact decimal a JSON number spells, including the scientific notation serde_json keeps verbatim
fn decimal(number: &Number, field: &'static str) -> Result<Decimal, ShadowEvalRequestError> {
    let token = number.to_string();
    Decimal::from_str(&token)
        .or_else(|_source| Decimal::from_scientific(&token))
        .map_err(|source| Validation {
            field,
            message: format!("{field} must be a decimal number: {source}"),
        })
}

fn ranged<T>(
    value: T,
    field: &'static str,
    (low, high): (T, T),
    message: &'static str,
) -> Result<T, ShadowEvalRequestError>
where
    T: Copy + PartialOrd,
{
    if !(low..=high).contains(&value) {
        return Err(Validation {
            field,
            message: message.to_owned(),
        });
    }
    Ok(value)
}

// LiteLLM dedupes with dict.fromkeys, so the first spelling of each value wins
pub(super) fn deduplicated(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
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
    given: Option<Vec<String>>,
    field: &'static str,
    max: usize,
) -> Result<Vec<String>, ShadowEvalRequestError> {
    let listed = given.unwrap_or_default();
    if listed.len() > max {
        return Err(Validation {
            field,
            message: format!("{field} must have at most {max} entries"),
        });
    }
    Ok(deduplicated(listed))
}

pub(super) fn percentage(given: Option<Number>) -> Result<f64, ShadowEvalRequestError> {
    let number = given.ok_or_else(|| Validation {
        field: "shadow_percentage",
        message: "shadow_percentage is required".to_owned(),
    })?;
    let parsed = number.as_f64().ok_or_else(|| Validation {
        field: "shadow_percentage",
        message: "shadow_percentage must be a finite number".to_owned(),
    })?;
    let checked = ranged(
        parsed,
        "shadow_percentage",
        (0.1_f64, 100.0_f64),
        "shadow_percentage must be between 0.1 and 100",
    )?;
    // Python's round(value, 2) is the correctly rounded two-decimal value of the same double
    format!("{checked:.2}")
        .parse::<f64>()
        .map_err(|source| Validation {
            field: "shadow_percentage",
            message: format!("shadow_percentage cannot be rounded: {source}"),
        })
}

pub(super) fn duration_days(number: &Number) -> Result<u32, ShadowEvalRequestError> {
    let days = integer(number, "duration_days")?;
    if !(1_i64..=30_i64).contains(&days) {
        return Err(Validation {
            field: "duration_days",
            message: "duration_days must be between 1 and 30".to_owned(),
        });
    }
    u32::try_from(days).map_err(|source| Validation {
        field: "duration_days",
        message: format!("duration_days is out of range: {source}"),
    })
}

pub(super) fn budget(number: &Number) -> Result<Decimal, ShadowEvalRequestError> {
    let amount = decimal(number, "max_budget")?;
    ranged(
        amount,
        "max_budget",
        (Decimal::new(1, 2), Decimal::new(10_000, 0)),
        "max_budget must be between 0.01 and 10000",
    )
}
